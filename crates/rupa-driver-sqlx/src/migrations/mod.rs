//! Migration sessions on [`SqlxExecutor`], from the engine protocols shared
//! with the native drivers (`rupa_migrate::protocol`): one generic [`Io`]
//! implementation, and per engine the `Session` and how typed user
//! statements are admitted.

#[cfg(feature = "mysql")]
mod mysql;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "_sqlite")]
mod sqlite;

use std::sync::Arc;
use std::time::Duration;

use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::AsyncTxConnection;
use rupa_core::{SqlType, Value};
use rupa_migrate::StateError;
use rupa_migrate::protocol::SqlState;
use rupa_migrate::protocol::asynchronous::Io;
use sqlx::Connection as _;

use crate::{Backend, SqlxError, SqlxExecutor, arguments};

/// An engine's migration state on a sqlx connection, and how a typed user
/// statement runs while a migration owns the connection.
pub trait Engine<DB: Backend>: Default + Send + Sync + 'static {
    /// Whether a run left state on the connection.
    fn is_active(&self) -> bool;

    /// Whether statements route through [`Engine::user_statement`].
    fn is_running(&self) -> bool;

    /// How long a runner waits for another runner's lock.
    fn lock_timeout(&mut self) -> &mut Duration;

    /// Runs a typed user statement under the engine's migration checks.
    fn user_statement<'e>(
        executor: &'e mut SqlxExecutor<DB>,
        statement: &'e Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, SqlxError>> + Send + 'e;
}

impl<DB: Backend> SqlxExecutor<DB> {
    /// Bounds how long a migration runner waits for another runner's lock:
    /// PostgreSQL's advisory lock (zero tries once; clamped to the
    /// `lock_timeout` maximum, about 24.8 days), MySQL's named lock (whole
    /// seconds, rounded up; zero tries once) or SQLite's writer lock (an
    /// existing durable owner fails immediately).
    pub fn with_migration_lock_timeout(mut self, timeout: Duration) -> Self {
        *self.migration.lock_timeout() = timeout;

        self
    }
}

impl From<StateError> for SqlxError {
    fn from(state: StateError) -> Self {
        Self::Migration(state)
    }
}

impl SqlState for SqlxError {
    fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Db(sqlx::Error::Database(error)) => error.code().map(String::from),
            _ => None,
        }
    }
}

impl<DB: Backend> Io for SqlxExecutor<DB> {
    type Error = SqlxError;
    type State = DB::Migration;

    fn state(&mut self) -> &mut DB::Migration {
        &mut self.migration
    }

    async fn query(
        &mut self,
        sql: &str,
        bound: &[Value],
        columns: &[SqlType],
    ) -> Result<Vec<Vec<Value>>, SqlxError> {
        let args = arguments::<DB>(bound)?;
        let fetched = DB::fetch(&mut self.conn, Arc::from(sql), args, None).await;
        let rows = fetched.map_err(|e| self.failed(e))?;

        rows.iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(index, want)| {
                        DB::read(row, index, *want).map_err(|e| SqlxError::Result(e.into()))
                    })
                    .collect()
            })
            .collect()
    }

    async fn execute(&mut self, sql: &str, bound: &[Value]) -> Result<u64, SqlxError> {
        let args = arguments::<DB>(bound)?;
        let result = DB::execute(&mut self.conn, Arc::from(sql), args).await;

        result.map_err(|e| self.failed(e))
    }

    async fn batch(&mut self, sql: &str) -> Result<(), SqlxError> {
        let result = DB::raw(&mut self.conn, sql).await;

        result.map_err(|e| self.failed(e))
    }

    async fn clean(&mut self) -> Result<(), SqlxError> {
        AsyncTxConnection::clean(self).await
    }

    async fn forget_statements(&mut self) -> Result<(), SqlxError> {
        let result = self.conn.clear_cached_statements().await;

        result.map_err(|e| self.failed(e))
    }
}
