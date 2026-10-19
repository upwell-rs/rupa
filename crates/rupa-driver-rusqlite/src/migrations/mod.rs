//! Migration control stays inside the native SQLite executor.

mod authorization;
mod error;
mod schema;
mod session;

use std::time::Duration;

use rupa_core::tx::TxConnection;
use rupa_migrate::Error;

use crate::{SqliteError, SqliteExecutor};

pub use error::MigrationStateError;

/// Private per-connection ownership and attempt state.
pub(super) struct State {
    pub(super) owner: Option<String>,
    attempt: Option<i64>,
    pub(super) poisoned: bool,
    lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            attempt: None,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl SqliteExecutor {
    /// Bounds migration writer-lock acquisition. An existing durable owner fails immediately.
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    fn validate_migration_connection(&mut self) -> Result<(), Error<SqliteError>> {
        self.clean().map_err(Error::Database)?;
        if !self.connection().is_autocommit() {
            return Err(Error::History(
                "connection is already inside an application transaction".into(),
            ));
        }

        let writable_schema: i64 = self
            .connection()
            .query_row("PRAGMA writable_schema", [], |r| r.get(0))
            .map_err(db)?;
        if writable_schema != 0 {
            return Err(Error::History(
                "writable_schema must be disabled on migration connections".into(),
            ));
        }

        let attached: i64 = self
            .connection()
            .query_row(
                "SELECT count(*) FROM pragma_database_list WHERE name NOT IN ('main', 'temp')",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        let temporary: i64 = self
            .connection()
            .query_row("SELECT count(*) FROM temp.sqlite_schema", [], |r| r.get(0))
            .map_err(db)?;
        if attached != 0 || temporary != 0 {
            return Err(Error::History(
                "migration sessions require only main and an empty temp namespace".into(),
            ));
        }

        Ok(())
    }

    fn verify_owner(&mut self) -> Result<(), Error<SqliteError>> {
        let owner =
            self.migration.owner.as_deref().ok_or_else(|| {
                Error::History("migration connection does not own the target".into())
            })?;
        let stored: Option<String> = self
            .conn
            .query_row(
                "SELECT owner FROM main._rupa_migration_meta WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(db)?;
        if stored.as_deref() != Some(owner) || self.migration.poisoned {
            return Err(Error::History(
                "migration ownership was lost or connection is uncertain".into(),
            ));
        }

        Ok(())
    }

    pub(super) fn execute_migration_statement(
        &mut self,
        statement: &rupa_core::ir::Statement,
        expect: rupa_core::query::Expect,
    ) -> Result<rupa_core::exec::Outcome, SqliteError> {
        self.user_executor()?.executor.run(statement, expect)
    }

    fn user_executor(&mut self) -> Result<authorization::Authorized<'_>, SqliteError> {
        let state = if self.migration.owner.is_none() {
            Some(MigrationStateError::NotOwner)
        } else if self.migration.poisoned {
            Some(MigrationStateError::Poisoned)
        } else if self.migration.attempt.is_none() {
            Some(MigrationStateError::NoAttempt)
        } else if self.connection().is_autocommit() {
            Some(MigrationStateError::NoTransaction)
        } else {
            None
        };

        if let Some(state) = state {
            return Err(SqliteError::Migration(state));
        }

        authorization::Authorized::new(self)
    }
}

fn db(error: rusqlite::Error) -> Error<SqliteError> {
    Error::Database(SqliteError::Db(error))
}
