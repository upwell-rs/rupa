//! Migration control stays inside the native MySQL executor. MySQL commits
//! DDL implicitly, so sessions run nontransactional bodies only; a named
//! session lock keeps runners exclusive across the per-artifact commits.

mod control;
mod session;
mod sql;

use std::time::Duration;

use mysql::prelude::Queryable;
use rupa_core::DialectId;
use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::TxConnection;
use rupa_migrate::{Engine, Error, StateError, Target};

use crate::{MySqlError, MySqlExecutor};

/// The `sql_mode` flag the migration script grammar assumes.
const NO_BACKSLASH_ESCAPES: &str = "NO_BACKSLASH_ESCAPES";

/// The named lock a connection holds and the session state it replaced.
#[derive(Debug)]
struct Owner {
    lock: String,
    sql_mode: String,
}

/// Per-connection ownership, attempt and body state.
#[derive(Debug)]
pub(super) struct State {
    owner: Option<Owner>,
    attempt: Option<i64>,
    body: bool,
    dispatched: bool,
    poisoned: bool,
    lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            attempt: None,
            body: false,
            dispatched: false,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl State {
    /// Whether a run left state on the connection: owned, mid-attempt,
    /// mid-body or poisoned. Pools must discard such connections.
    pub(super) fn is_active(&self) -> bool {
        self.owner.is_some() || self.attempt.is_some() || self.body || self.poisoned
    }

    /// Whether typed queries go through the migration checks.
    pub(super) fn is_running(&self) -> bool {
        self.owner.is_some() || self.poisoned
    }

    /// Whether user statements may run: owned, unpoisoned, inside a begun attempt body.
    fn user_state(&self) -> Result<(), StateError> {
        if self.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else if !self.body {
            Err(StateError::NoTransaction)
        } else {
            Ok(())
        }
    }

    /// `GET_LOCK` waits whole seconds: a nonzero timeout rounds up, zero tries once.
    fn lock_seconds(&self) -> u64 {
        self.lock_timeout.as_secs() + u64::from(self.lock_timeout.subsec_nanos() > 0)
    }
}

/// What a clean migration connection reports about itself.
#[derive(Debug)]
struct Connection {
    database: String,
    sql_mode: String,
}

impl Connection {
    /// The named lock of this target: the selected database is both the
    /// database and the owned namespace.
    fn lock(&self) -> String {
        Target {
            engine: Engine::MySql,
            database: &self.database,
            namespace: &self.database,
        }
        .lock_name()
    }

    /// The session `sql_mode` with `NO_BACKSLASH_ESCAPES` added.
    fn migration_sql_mode(&self) -> String {
        let present = self
            .sql_mode
            .split(',')
            .any(|mode| mode.eq_ignore_ascii_case(NO_BACKSLASH_ESCAPES));

        match (present, self.sql_mode.is_empty()) {
            (true, _) => self.sql_mode.clone(),
            (false, true) => NO_BACKSLASH_ESCAPES.to_owned(),
            (false, false) => format!("{},{NO_BACKSLASH_ESCAPES}", self.sql_mode),
        }
    }
}

impl MySqlExecutor {
    /// Bounds how long a runner waits for another runner's named lock, in
    /// whole seconds rounded up; zero tries once without waiting.
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    /// Reads the connection, rejecting one inside a transaction or without a
    /// selected database.
    fn describe_connection(&mut self) -> Result<Connection, Error<MySqlError>> {
        self.clean().map_err(Error::Database)?;

        let in_transaction =
            || Error::History("connection is already inside an application transaction".into());
        let row: Option<(i64, Option<String>, String)> =
            self.conn.query_first(sql::DESCRIBE).map_err(db)?;
        let Some((autocommit, database, sql_mode)) = row else {
            return Err(Error::History(
                "connection did not describe its database".into(),
            ));
        };

        if autocommit != 1 {
            return Err(in_transaction());
        }

        match self.conn.query_drop(sql::TRANSACTION_PROBE) {
            Err(mysql::Error::MySqlError(error)) if error.code == sql::IN_TRANSACTION => {
                return Err(in_transaction());
            }
            probed => probed.map_err(db)?,
        }

        let Some(database) = database else {
            return Err(Error::History(
                "migration connections require a selected database".into(),
            ));
        };

        Ok(Connection { database, sql_mode })
    }

    /// Verifies the runner still holds its named lock on this session.
    fn verify_owner(&mut self) -> Result<(), Error<MySqlError>> {
        let lost =
            || Error::History("migration ownership was lost or connection is uncertain".into());
        let Some(owner) = self
            .migration
            .owner
            .as_ref()
            .filter(|_| !self.migration.poisoned)
        else {
            return Err(lost());
        };
        let held: Option<i64> = self
            .conn
            .exec_first(sql::HOLDS_LOCK, (owner.lock.as_str(),))
            .map_err(db)?;

        if held != Some(1) {
            self.migration.poisoned = true;

            return Err(lost());
        }

        Ok(())
    }

    /// Typed queries in a migration are refused by the same grammar as
    /// scripts, since MySQL has no client-side authorizer.
    pub(super) fn execute_migration_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, MySqlError> {
        self.migration.user_state().map_err(MySqlError::Migration)?;

        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;

        rupa_migrate::validate_script(&rendered.sql, DialectId::MySql)
            .map_err(|e| MySqlError::Migration(StateError::Script(e)))?;

        self.migration.dispatched = true;

        self.run(statement, expect)
    }
}

/// A driver failure as a migration error.
fn db(error: mysql::Error) -> Error<MySqlError> {
    Error::Database(MySqlError::Db(error))
}
