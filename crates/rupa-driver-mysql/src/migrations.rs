//! The sync migration session on [`MySqlExecutor`], from the shared MySQL
//! protocol.

use std::time::Duration;

use mysql::prelude::Queryable;
use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::TxConnection;
use rupa_core::{SqlType, Value};
use rupa_migrate::StateError;
use rupa_migrate::protocol::SqlState;
use rupa_migrate::protocol::mysql::State;
use rupa_migrate::protocol::sync::Io;

use crate::value::{from_mysql, to_mysql};
use crate::{MySqlError, MySqlExecutor};

rupa_migrate::__session!(sync, mysql, MySqlExecutor);

impl From<StateError> for MySqlError {
    fn from(state: StateError) -> Self {
        Self::Migration(state)
    }
}

impl SqlState for MySqlError {
    fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Db(mysql::Error::MySqlError(error)) => Some(error.state.clone()),
            _ => None,
        }
    }
}

/// `values` as positional parameters.
fn params(values: &[Value]) -> Result<mysql::Params, MySqlError> {
    if values.is_empty() {
        return Ok(mysql::Params::Empty);
    }

    let positional = values.iter().map(to_mysql).collect::<Result<_, _>>()?;

    Ok(mysql::Params::Positional(positional))
}

impl Io for MySqlExecutor {
    type Error = MySqlError;
    type State = State;

    fn state(&mut self) -> &mut State {
        &mut self.migration
    }

    fn query(
        &mut self,
        sql: &str,
        bound: &[Value],
        columns: &[SqlType],
    ) -> Result<Vec<Vec<Value>>, MySqlError> {
        let rows: Vec<mysql::Row> = self.conn.exec(sql, params(bound)?)?;

        rows.into_iter()
            .map(|row| {
                let values = row.unwrap();

                columns
                    .iter()
                    .zip(&values)
                    .map(|(want, value)| {
                        from_mysql(value, *want).map_err(|e| MySqlError::Result(e.into()))
                    })
                    .collect()
            })
            .collect()
    }

    fn execute(&mut self, sql: &str, bound: &[Value]) -> Result<u64, MySqlError> {
        self.conn.exec_drop(sql, params(bound)?)?;

        Ok(self.conn.affected_rows())
    }

    fn batch(&mut self, sql: &str) -> Result<(), MySqlError> {
        Ok(self.conn.query_drop(sql)?)
    }

    fn clean(&mut self) -> Result<(), MySqlError> {
        TxConnection::clean(self)
    }
}

impl MySqlExecutor {
    /// Bounds how long a runner waits for another runner's named lock, in
    /// whole seconds rounded up; zero tries once without waiting.
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    /// Typed queries in a migration are refused by the same grammar as
    /// scripts, since MySQL has no client-side authorizer.
    pub(super) fn execute_migration_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, MySqlError> {
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;

        self.migration.user_statement(&rendered.sql)?;

        self.dispatch(statement, expect)
    }
}
