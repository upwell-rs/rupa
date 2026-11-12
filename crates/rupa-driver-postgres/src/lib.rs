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
//! [`PgExecutor`] is a [`TxConnection`] and [`Transactional`]: `begin`
//! returns a [`PgTx`], itself an executor, and `begin` on a `PgTx` opens a
//! savepoint. A `PgTx` dropped without `commit`/`rollback` rolls back
//! immediately. If that rollback fails (e.g. the connection broke), the
//! executor is marked dirty, and the rollback is retried before the next
//! statement.
//!
//! # Sharing
//!
//! A `postgres::Client` runs one statement at a time through `&mut`, so the
//! executor holds it behind a mutex, taken for each statement. That makes a
//! single executor, and a transaction on it, shareable by reference; prefer
//! [`PgPool`] when several threads query concurrently.

#![warn(missing_docs)]

#[cfg(feature = "dynamic")]
mod driver;
#[cfg(feature = "migrations")]
mod migrations;
#[cfg(feature = "pool")]
pub mod pool;

use postgres::fallible_iterator::FallibleIterator;
use postgres::types::ToSql;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rupa_core::Postgres;
use rupa_core::exec::{Engine, Executor, Immediate, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Transactional, Tx, TxConnection, TxOptions, TxSlot};
use rupa_driver_tokio_postgres::shared::{Param, PgRows};

#[cfg(feature = "dynamic")]
pub use driver::PgDriver;
#[cfg(feature = "pool")]
pub use pool::{PgConn, PgManager, PgPool, PgPoolTx};
pub use rupa_driver_tokio_postgres::{PgError, STATEMENT_CACHE_CAPACITY, StatementCache};

/// A transaction (or savepoint) on a [`PgExecutor`]. Rolled back when dropped
/// without `commit`.
pub type PgTx<'t> = Tx<&'t PgExecutor>;

/// Executes queries on a `postgres::Client`, one statement at a time.
pub struct PgExecutor {
    client: Mutex<postgres::Client>,
    dialect: Postgres,
    pending: PendingRollback,
    slot: TxSlot,
    statements: StatementCache,
    #[cfg(feature = "migrations")]
    migration: rupa_migrate::protocol::postgres::State,
}

impl PgExecutor {
    /// Wraps a connected client.
    pub fn new(client: postgres::Client) -> Self {
        Self {
            client: Mutex::new(client),
            dialect: Postgres,
            pending: PendingRollback::default(),
            slot: TxSlot::default(),
            statements: StatementCache::default(),
            #[cfg(feature = "migrations")]
            migration: rupa_migrate::protocol::postgres::State::default(),
        }
    }

    /// Keeps at most `capacity` prepared statements (`0`: prepare every
    /// query anew).
    pub fn with_statement_cache_size(self, capacity: usize) -> Self {
        self.statements.set_capacity(capacity);
        self
    }

    /// The underlying client. Statements sent through it directly bypass the
    /// pending-rollback check; call [`clean`](TxConnection::clean) first.
    pub fn client(&mut self) -> &mut postgres::Client {
        self.client
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The underlying client.
    pub fn into_inner(self) -> postgres::Client {
        self.client
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The prepared-statement cache, e.g. to clear it after changing the
    /// schema through [`client`](Self::client).
    pub fn statement_cache(&self) -> &StatementCache {
        &self.statements
    }

    /// The client, for one statement. A statement that panicked leaves no
    /// state behind the lock (a transaction it was in rolls back when
    /// unwinding drops it), so a poisoned lock is still usable.
    fn lock(&self) -> MutexGuard<'_, postgres::Client> {
        self.client.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The prepared statement for `sql`, from the cache or prepared now.
    fn prepare(
        &self,
        client: &mut postgres::Client,
        sql: &Arc<str>,
    ) -> Result<postgres::Statement, PgError> {
        if let Some(statement) = self.statements.get(sql) {
            return Ok(statement);
        }

        let statement = client.prepare(sql)?;

        self.statements.insert(sql.clone(), statement.clone());

        Ok(statement)
    }

    /// Rendered SQL is cached by statement shape, prepared statements by
    /// SQL text: a repeated query skips both rendering and parsing.
    ///
    /// The statement may be stale; prepare it afresh next time.
    fn dispatch(&self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        let mut client = self.lock();

        self.clean_on(&mut client)?;

        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let prepared = self.prepare(&mut client, &rendered.sql)?;
        let params: Vec<Param<'_>> = rendered.params.iter().map(Param).collect();
        let result = query(&mut client, &prepared, &params, expect);

        if result.is_err() {
            self.statements.evict(&rendered.sql);
        }

        result
    }

    /// Sends the rollback a dropped transaction owes, on the locked `client`.
    fn clean_on(&self, client: &mut postgres::Client) -> Result<(), PgError> {
        if let Some(statement) = self.pending.statement() {
            client.batch_execute(&rupa_sql::render_tx(&statement, &self.dialect)?)?;
            self.pending.clear(statement);
        }

        Ok(())
    }
}

/// Runs a prepared `statement` with `params` on `client`.
fn query(
    client: &mut postgres::Client,
    statement: &postgres::Statement,
    params: &[Param<'_>],
    expect: Expect,
) -> Result<Outcome, PgError> {
    let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as &(dyn ToSql + Sync)).collect();

    if expect == Expect::Affected {
        return Ok(Outcome::Affected(client.execute(statement, &refs)?));
    }

    let rows = match expect.fetch_limit() {
        None => client.query(statement, &refs)?,
        Some(limit) => client.query_raw(statement, refs)?.take(limit).collect()?,
    };

    Ok(Outcome::Rows(Box::new(PgRows::new(rows))))
}

/// Whether the client is closed is left out while a statement holds it.
impl std::fmt::Debug for PgExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let closed = self.client.try_lock().ok().map(|client| client.is_closed());

        f.debug_struct("PgExecutor")
            .field("closed", &closed)
            .field("dirty", &self.pending.is_pending())
            .field("cached_statements", &self.statements.len())
            .finish_non_exhaustive()
    }
}

impl Engine for PgExecutor {
    type Completion = Immediate;
}

impl Executor for PgExecutor {
    type Dialect = Postgres;
    type Error = PgError;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, PgError> {
        #[cfg(feature = "migrations")]
        self.migration.outside_run()?;

        self.dispatch(statement, expect)
    }
}

impl TxConnection for PgExecutor {
    fn control(&self, statement: TxStatement) -> Result<(), PgError> {
        let mut client = self.lock();

        self.clean_on(&mut client)?;

        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;

        Ok(client.batch_execute(&sql)?)
    }

    fn defer_rollback(&self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&self) -> Result<(), PgError> {
        if !self.pending.is_pending() {
            return Ok(());
        }

        self.clean_on(&mut self.lock())
    }

    fn slot(&self) -> &TxSlot {
        &self.slot
    }
}

impl Transactional for PgExecutor {
    type Tx<'t> = PgTx<'t>;

    fn begin_with(&self, options: TxOptions) -> Result<PgTx<'_>, PgError> {
        Tx::begin(self, options)
    }
}
