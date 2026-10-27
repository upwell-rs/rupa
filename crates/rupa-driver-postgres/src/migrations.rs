//! The sync migration session on [`PgExecutor`], from the shared PostgreSQL
//! protocol.

use postgres::types::ToSql;
use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_core::tx::TxConnection;
use rupa_core::{SqlType, Value};
use rupa_driver_tokio_postgres::protocol::{params, values};
use rupa_migrate::protocol::postgres::State;
use rupa_migrate::protocol::sync::Io;

use crate::{PgError, PgExecutor};

rupa_migrate::__session!(sync, postgres, PgExecutor);

impl Io for PgExecutor {
    type Error = PgError;
    type State = State;

    fn state(&mut self) -> &mut State {
        &mut self.migration
    }

    fn query(
        &mut self,
        sql: &str,
        bound: &[Value],
        columns: &[SqlType],
    ) -> Result<Vec<Vec<Value>>, PgError> {
        let params = params(bound);
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let rows = self.client.query(sql, &refs)?;

        values(&rows, columns)
    }

    fn execute(&mut self, sql: &str, bound: &[Value]) -> Result<u64, PgError> {
        let params = params(bound);
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

        Ok(self.client.execute(sql, &refs)?)
    }

    fn batch(&mut self, sql: &str) -> Result<(), PgError> {
        Ok(self.client.batch_execute(sql)?)
    }

    fn clean(&mut self) -> Result<(), PgError> {
        TxConnection::clean(self)
    }

    fn forget_statements(&mut self) -> Result<(), PgError> {
        self.statements.clear();

        Ok(())
    }
}

impl PgExecutor {
    /// Bounds how long a runner waits for another runner's advisory lock.
    /// Zero tries once without waiting; waits beyond PostgreSQL's
    /// `lock_timeout` maximum (about 24.8 days) are clamped to it.
    pub fn with_migration_lock_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.migration.lock_timeout = timeout;

        self
    }

    /// Typed queries in a migration are refused by the same grammar as
    /// scripts, since Postgres has no client-side authorizer.
    pub(crate) fn execute_migration_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, PgError> {
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;

        self.migration.user_statement(&rendered.sql)?;

        self.dispatch(statement, expect)
    }
}
