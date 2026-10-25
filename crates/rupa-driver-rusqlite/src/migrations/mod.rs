//! The sync migration session on [`SqliteExecutor`], from the shared SQLite
//! protocol.

mod authorization;

use std::time::Duration;

use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::TxConnection;
use rupa_core::{SqlType, Value};
use rupa_migrate::StateError;
use rupa_migrate::protocol::SqlState;
use rupa_migrate::protocol::sqlite::{self, State};
use rupa_migrate::protocol::sync::{Io, SqliteIo};
use rusqlite::types::Value as SqlValue;

use crate::value::{Param, from_sql};
use crate::{SqliteError, SqliteExecutor};

rupa_migrate::__session!(sync, sqlite, SqliteExecutor);

impl From<StateError> for SqliteError {
    fn from(state: StateError) -> Self {
        Self::Migration(state)
    }
}

/// SQLite reports result codes, not SQLSTATEs.
impl SqlState for SqliteError {
    fn sqlstate(&self) -> Option<String> {
        None
    }
}

impl Io for SqliteExecutor {
    type Error = SqliteError;
    type State = State;

    fn state(&mut self) -> &mut State {
        &mut self.migration
    }

    fn query(
        &mut self,
        sql: &str,
        bound: &[Value],
        columns: &[SqlType],
    ) -> Result<Vec<Vec<Value>>, SqliteError> {
        let mut statement = self.conn.prepare(sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(bound.iter().map(Param)))?;
        let mut out = Vec::new();

        while let Some(row) = rows.next()? {
            let values = columns
                .iter()
                .enumerate()
                .map(|(index, want)| {
                    let value = row.get::<_, SqlValue>(index)?;

                    from_sql(&value, *want, index).map_err(|e| SqliteError::Result(e.into()))
                })
                .collect::<Result<_, _>>()?;

            out.push(values);
        }

        Ok(out)
    }

    fn execute(&mut self, sql: &str, bound: &[Value]) -> Result<u64, SqliteError> {
        let count = self
            .conn
            .execute(sql, rusqlite::params_from_iter(bound.iter().map(Param)))?;

        Ok(count as u64)
    }

    fn batch(&mut self, sql: &str) -> Result<(), SqliteError> {
        Ok(self.conn.execute_batch(sql)?)
    }

    fn clean(&mut self) -> Result<(), SqliteError> {
        TxConnection::clean(self)
    }
}

impl SqliteIo for SqliteExecutor {
    fn is_autocommit(&mut self) -> Result<bool, SqliteError> {
        Ok(self.conn.is_autocommit())
    }

    fn authorizer(&mut self, enabled: bool) -> Result<(), SqliteError> {
        authorization::install(&mut self.conn, enabled)
    }
}

impl SqliteExecutor {
    /// Bounds migration writer-lock acquisition. An existing durable owner fails immediately.
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    /// Runs a typed user statement under the migration authorizer.
    pub(super) fn execute_migration_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, SqliteError> {
        sqlite::sync::user_begin(self)?;

        let result = self.run(statement, expect);
        let ended = sqlite::sync::user_end(self);

        result.and_then(|outcome| ended.map(|()| outcome))
    }
}
