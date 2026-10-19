//! Migration control stays inside the native Postgres executor.

mod schema;
mod session;

use std::time::Duration;

use postgres::SimpleQueryMessage;
use rupa_core::DialectId;
use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::TxConnection;
use rupa_migrate::{Engine, Error, StateError, Target};

use crate::{PgError, PgExecutor};

/// The advisory lock this connection holds and the namespace it owns.
#[derive(Debug)]
pub(super) struct Owner {
    key: i64,
    namespace: String,
}

/// Private per-connection ownership, attempt and transaction state.
pub(super) struct State {
    pub(super) owner: Option<Owner>,
    attempt: Option<i64>,
    transaction: Option<i64>,
    pub(super) poisoned: bool,
    lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            attempt: None,
            transaction: None,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

/// What a clean migration connection reports about itself.
struct Connection {
    database: String,
    namespace: String,
}

impl PgExecutor {
    /// Bounds how long a runner waits for another runner's advisory lock.
    /// Zero tries once without waiting.
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    /// Rejects connections inside an application transaction or with
    /// settings the script grammar does not assume. Uses the simple query
    /// protocol, where an implicit transaction starts with its statement.
    fn validate_migration_connection(&mut self) -> Result<Connection, Error<PgError>> {
        let sql = "SELECT transaction_timestamp() = statement_timestamp(), current_setting('standard_conforming_strings'), current_database(), current_schema()";
        let mut values = Vec::new();

        self.clean().map_err(Error::Database)?;
        for message in self.client.simple_query(sql).map_err(db)? {
            if let SimpleQueryMessage::Row(row) = message {
                values = (0..4).map(|i| row.get(i).map(str::to_owned)).collect();
            }
        }

        let [autocommit, standard, Some(database), namespace] = values.as_slice() else {
            return Err(Error::History(
                "connection did not describe its database".into(),
            ));
        };

        if autocommit.as_deref() != Some("t") {
            return Err(Error::History(
                "connection is already inside an application transaction".into(),
            ));
        }

        if standard.as_deref() != Some("on") {
            return Err(Error::History(
                "migration connections require standard_conforming_strings = on".into(),
            ));
        }

        let Some(namespace) = namespace else {
            return Err(Error::History(
                "migration connections require a current schema on the search_path".into(),
            ));
        };

        Ok(Connection {
            database: database.clone(),
            namespace: namespace.clone(),
        })
    }

    fn target_key(connection: &Connection) -> i64 {
        Target {
            engine: Engine::Postgres,
            database: &connection.database,
            namespace: &connection.namespace,
        }
        .advisory_key()
    }

    /// Verifies the runner still holds its advisory lock on this session.
    fn verify_owner(&mut self) -> Result<(), Error<PgError>> {
        let lost =
            || Error::History("migration ownership was lost or connection is uncertain".into());
        let key = match &self.migration.owner {
            Some(owner) if !self.migration.poisoned => owner.key,
            _ => return Err(lost()),
        };
        let held: bool = self
            .client
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid() AND granted AND objsubid = 1 AND classid::bigint = $1 AND objid::bigint = $2)",
                &[&i64::from((key >> 32) as u32), &i64::from(key as u32)],
            )
            .map_err(db)?
            .get(0);

        if !held {
            self.migration.poisoned = true;

            return Err(lost());
        }

        Ok(())
    }

    fn owned_namespace(&self) -> Result<String, Error<PgError>> {
        self.migration
            .owner
            .as_ref()
            .map(|owner| owner.namespace.clone())
            .ok_or_else(|| Error::History("migration connection does not own the target".into()))
    }

    /// The qualified name of a reserved metadata table in the owned namespace.
    fn reserved(&self, table: &str) -> Result<String, Error<PgError>> {
        Ok(format!("{}.{table}", quote(&self.owned_namespace()?)))
    }

    fn user_state(&self) -> Result<(), StateError> {
        if self.migration.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.migration.poisoned {
            Err(StateError::Poisoned)
        } else if self.migration.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else if self.migration.transaction.is_none() {
            Err(StateError::NoTransaction)
        } else {
            Ok(())
        }
    }

    /// Typed queries in a migration are refused by the same grammar as
    /// scripts, since Postgres has no client-side authorizer.
    pub(super) fn execute_migration_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, PgError> {
        self.user_state().map_err(PgError::Migration)?;

        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;

        rupa_migrate::validate_script(&rendered.sql, DialectId::Postgres)
            .map_err(|e| PgError::Migration(StateError::Script(e)))?;

        self.run(statement, expect)
    }
}

/// Quotes an identifier for interpolation into framework-owned SQL.
fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn db(error: postgres::Error) -> Error<PgError> {
    Error::Database(PgError::Db(error))
}
