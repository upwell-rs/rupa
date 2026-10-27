//! Async RUPA executor on [`tokio_postgres`].
//!
//! - [`PgExecutor`] wraps one `tokio_postgres::Client`.
//! - With the `pool` feature, `PgPool` pools them (on `deadpool`).
//! - With the `dynamic` feature, `PgDriver` registers the driver for
//!   `postgres:` URLs (see `rupa_core::registry`).
//! - With `tls-rustls`, `PgPool::connect` negotiates TLS on rustls as the
//!   URL's `sslmode` asks.
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
//! [`PgExecutor`] is an [`AsyncTxConnection`], so it is
//! [`AsyncTransactional`](rupa_core::tx::AsyncTransactional): `begin` returns
//! a [`PgTx`], itself an executor, and `begin` on a `PgTx` opens a savepoint.
//! Rust has no async `Drop`, so a `PgTx` dropped without `commit`/`rollback`
//! cannot roll back on the spot. Instead it marks the executor dirty, and the
//! rollback is sent before the connection's next statement; `PgPool` also
//! cleans dirty connections when they are returned.

#![warn(missing_docs)]

#[cfg(feature = "dynamic")]
mod driver;
#[cfg(feature = "migrations")]
mod migration_session;
#[cfg(feature = "pool")]
pub mod pool;
#[cfg(feature = "migration-protocol")]
#[doc(hidden)]
pub mod protocol;
#[doc(hidden)]
pub mod shared;

use futures_util::{StreamExt, TryStreamExt, pin_mut};
use postgres_types::ToSql;
use std::sync::Arc;

use rupa_core::Postgres;
use rupa_core::exec::{AsyncExecutor, Awaited, Engine, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{AsyncTx, AsyncTxConnection, PendingRollback};

#[cfg(feature = "dynamic")]
pub use driver::PgDriver;
#[cfg(feature = "pool")]
pub use pool::{PgConn, PgManager, PgPool, PgPoolTx};
use shared::{Param, PgRows};
pub use shared::{PgError, STATEMENT_CACHE_CAPACITY, StatementCache};

/// A transaction (or savepoint) on a [`PgExecutor`]. Dropped while open, it
/// leaves the rollback to the executor (see the crate docs).
pub type PgTx<'t> = AsyncTx<&'t mut PgExecutor>;

/// Executes queries on a `tokio_postgres::Client`.
pub struct PgExecutor {
    client: tokio_postgres::Client,
    dialect: Postgres,
    pending: PendingRollback,
    statements: StatementCache,
    #[cfg(feature = "migrations")]
    migration: rupa_migrate::protocol::postgres::State,
}

impl PgExecutor {
    /// Wraps a connected client. The client's connection future must be
    /// driven (usually `tokio::spawn(connection)`).
    pub fn new(client: tokio_postgres::Client) -> Self {
        Self {
            client,
            dialect: Postgres,
            pending: PendingRollback::default(),
            statements: StatementCache::default(),
            #[cfg(feature = "migrations")]
            migration: rupa_migrate::protocol::postgres::State::default(),
        }
    }

    /// Keeps at most `capacity` prepared statements (`0`: prepare every
    /// query anew).
    pub fn with_statement_cache_size(mut self, capacity: usize) -> Self {
        self.statements.set_capacity(capacity);
        self
    }

    /// The underlying client. Statements sent through it directly bypass the
    /// pending-rollback check; call [`clean`](AsyncTxConnection::clean) first.
    pub fn client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    /// The underlying client.
    pub fn into_inner(self) -> tokio_postgres::Client {
        self.client
    }

    /// The prepared-statement cache, e.g. to clear it after changing the
    /// schema through [`client`](Self::client).
    pub fn statement_cache(&mut self) -> &mut StatementCache {
        &mut self.statements
    }

    /// The prepared statement for `sql`, from the cache or prepared now.
    async fn prepare(&mut self, sql: &Arc<str>) -> Result<tokio_postgres::Statement, PgError> {
        if let Some(statement) = self.statements.get(sql) {
            return Ok(statement);
        }

        let statement = self.client.prepare(sql).await?;
        self.statements.insert(sql.clone(), statement.clone());
        Ok(statement)
    }

    /// Rendered SQL is cached by statement shape, prepared statements by
    /// SQL text: a repeated query skips both rendering and parsing.
    ///
    /// The statement may be stale; prepare it afresh next time.
    async fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        self.clean().await?;
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let prepared = self.prepare(&rendered.sql).await?;
        let params: Vec<Param<'_>> = rendered.params.iter().map(Param).collect();
        let result = self.query(&prepared, &params, expect).await;
        if result.is_err() {
            self.statements.evict(&rendered.sql);
        }

        result
    }

    async fn query(
        &self,
        statement: &tokio_postgres::Statement,
        params: &[Param<'_>],
        expect: Expect,
    ) -> Result<Outcome, PgError> {
        let refs: Vec<&(dyn ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        if expect == Expect::Affected {
            return Ok(Outcome::Affected(
                self.client.execute(statement, &refs).await?,
            ));
        }

        let rows = match expect.fetch_limit() {
            None => self.client.query(statement, &refs).await?,
            Some(limit) => {
                let stream = self.client.query_raw(statement, refs).await?;
                pin_mut!(stream);
                stream.take(limit).try_collect().await?
            }
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

impl Engine for PgExecutor {
    type Completion = Awaited;
}

impl AsyncExecutor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    async fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        #[cfg(feature = "migrations")]
        if self.migration.is_running() {
            return self.execute_migration_statement(statement, expect).await;
        }

        self.run(statement, expect).await
    }
}

impl AsyncTxConnection for PgExecutor {
    async fn control(&mut self, statement: TxStatement) -> Result<(), PgError> {
        self.clean().await?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.client.batch_execute(&sql).await?)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    async fn clean(&mut self) -> Result<(), PgError> {
        if let Some(statement) = self.pending.statement() {
            self.client
                .batch_execute(&rupa_sql::render_tx(&statement, &self.dialect)?)
                .await?;
            self.pending.clear();
        }

        Ok(())
    }
}
