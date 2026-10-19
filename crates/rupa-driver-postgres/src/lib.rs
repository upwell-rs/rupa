//! Synchronous RUPA executor on the [`postgres`] crate.
//!
//! - [`PgExecutor`] wraps one `postgres::Client`.
//! - With the `pool` feature, [`PgPool`] pools them (on `r2d2`).
//! - With the `dynamic` feature, `PgDriver` registers the driver as
//!   `postgres-sync` (see `rupa_core::registry`; select it with
//!   `driver = "postgres-sync"`, since the async driver is preferred for
//!   `postgres:` URLs).
//!
//! # Performance
//!
//! Two caches, so a repeated query neither renders nor parses again:
//!
//! - rendered SQL, by statement shape, process-wide (`rupa_sql::cache`);
//! - server-side prepared statements, by SQL text, per connection
//!   ([`StatementCache`], [`STATEMENT_CACHE_CAPACITY`] by default; see
//!   [`PgExecutor::with_statement_cache_size`]). A cached query costs one
//!   round trip instead of two.
//!
//! # Transactions
//!
//! [`PgExecutor`] is a [`TxConnection`], so it is
//! [`Transactional`](rupa_core::tx::Transactional): `begin` returns a
//! [`PgTx`], itself an executor, and `begin` on a `PgTx` opens a savepoint. A
//! `PgTx` dropped without `commit`/`rollback` rolls back immediately. If that
//! rollback fails (e.g. the connection broke), the executor is marked dirty,
//! and the rollback is retried before the next statement.

#![warn(missing_docs)]

#[cfg(feature = "dynamic")]
mod driver;
#[cfg(feature = "migrations")]
mod migrations;
#[cfg(feature = "pool")]
pub mod pool;

use postgres::fallible_iterator::FallibleIterator;
use postgres::types::ToSql;
use std::sync::Arc;

use rupa_core::Postgres;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Tx, TxConnection};
use rupa_driver_tokio_postgres::shared::{Param, PgRows};

#[cfg(feature = "dynamic")]
pub use driver::PgDriver;
#[cfg(feature = "pool")]
pub use pool::{PgConn, PgManager, PgPool, PgPoolTx};
pub use rupa_driver_tokio_postgres::{PgError, STATEMENT_CACHE_CAPACITY, StatementCache};

/// A transaction (or savepoint) on a [`PgExecutor`]. Rolled back when dropped
/// without `commit`.
pub type PgTx<'t> = Tx<&'t mut PgExecutor>;

/// Executes queries on a `postgres::Client`.
pub struct PgExecutor {
    client: postgres::Client,
    dialect: Postgres,
    pending: PendingRollback,
    statements: StatementCache,
    #[cfg(feature = "migrations")]
    migration: migrations::State,
}

impl PgExecutor {
    /// Wraps a connected client.
    pub fn new(client: postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
            pending: PendingRollback::default(),
            statements: StatementCache::default(),
            #[cfg(feature = "migrations")]
            migration: migrations::State::default(),
        }
    }

    /// Keeps at most `capacity` prepared statements (`0`: prepare every
    /// query anew).
    pub fn with_statement_cache_size(mut self, capacity: usize) -> Self {
        self.statements.set_capacity(capacity);
        self
    }

    /// The underlying client. Statements sent through it directly bypass the
    /// pending-rollback check; call [`clean`](TxConnection::clean) first.
    pub fn client(&mut self) -> &mut postgres::Client {
        &mut self.client
    }

    /// The underlying client.
    pub fn into_inner(self) -> postgres::Client {
        self.client
    }

    /// The prepared-statement cache, e.g. to clear it after changing the
    /// schema through [`client`](Self::client).
    pub fn statement_cache(&mut self) -> &mut StatementCache {
        &mut self.statements
    }

    /// The prepared statement for `sql`, from the cache or prepared now.
    fn prepare(&mut self, sql: &Arc<str>) -> Result<postgres::Statement, PgError> {
        if let Some(statement) = self.statements.get(sql) {
            return Ok(statement);
        }

        let statement = self.client.prepare(sql)?;
        self.statements.insert(sql.clone(), statement.clone());
        Ok(statement)
    }

    /// Rendered SQL is cached by statement shape, prepared statements by
    /// SQL text: a repeated query skips both rendering and parsing.
    ///
    /// The statement may be stale; prepare it afresh next time.
    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.clean()?;
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let prepared = self.prepare(&rendered.sql)?;
        let params: Vec<Param<'_>> = rendered.params.iter().map(Param).collect();
        let result = self.query(&prepared, &params, expect);
        if result.is_err() {
            self.statements.evict(&rendered.sql);
        }

        result
    }

    fn query(
        &mut self,
        statement: &postgres::Statement,
        params: &[Param<'_>],
        expect: Expect,
    ) -> Result<Outcome, PgError> {
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        if expect == Expect::Affected {
            return Ok(Outcome::Affected(self.client.execute(statement, &refs)?));
        }

        let rows = match expect.fetch_limit() {
            None => self.client.query(statement, &refs)?,
            Some(limit) => self
                .client
                .query_raw(statement, refs)?
                .take(limit)
                .collect()?,
        };
        Ok(Outcome::Rows(Box::new(PgRows::new(rows))))
    }
}

impl std::fmt::Debug for PgExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgExecutor")
            .field("closed", &self.client.is_closed())
            .field("dirty", &self.pending.is_pending())
            .field("cached_statements", &self.statements.len())
            .finish_non_exhaustive()
    }
}

impl Executor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        #[cfg(feature = "migrations")]
        if self.migration.owner.is_some() || self.migration.poisoned {
            return self.execute_migration_statement(statement, expect);
        }

        self.run(statement, expect)
    }
}

impl TxConnection for PgExecutor {
    fn control(&mut self, statement: TxStatement) -> Result<(), PgError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.client.batch_execute(&sql)?)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&mut self) -> Result<(), PgError> {
        if let Some(statement) = self.pending.statement() {
            self.client
                .batch_execute(&rupa_sql::render_tx(&statement, &self.dialect)?)?;
            self.pending.clear();
        }

        Ok(())
    }
}
