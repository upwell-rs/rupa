//! Synchronous RUPA executor on the [`mysql`] crate (MySQL 8.0+).
//!
//! - [`MySqlExecutor`] wraps one `mysql::Conn`.
//! - With the `pool` feature, [`MySqlPool`] pools them (on `r2d2`).
//! - With the `dynamic` feature, `MySqlDriver` registers the driver for
//!   `mysql:` URLs (see `rupa_core::registry`).
//!
//! For async code, prefer the sqlx-based driver (`rupa-driver-sqlx`, feature
//! `mysql`): this one blocks the calling thread for each round trip.
//!
//! # Semantics
//!
//! RUPA reproduces Postgres semantics on every backend. For MySQL that needs
//! one connection setting: `UPDATE` must report *matched* rows, not changed
//! ones (`CLIENT_FOUND_ROWS`), so that `bool`/count results of an update mean
//! the same as on Postgres. [`MySqlExecutor::connect`] and the pool set it;
//! a connection passed to [`MySqlExecutor::new`] must have been opened with
//! it ([`found_rows`] adds it to your options).
//!
//! # Transactions
//!
//! [`MySqlExecutor`] is a [`TxConnection`] and [`Transactional`]: `begin`
//! returns a [`MySqlTx`]; nested `begin` opens a savepoint; a dropped
//! `MySqlTx` rolls back immediately (or, if that fails, before the next
//! statement). Transaction-control statements that render as several
//! statements (an isolation level plus `START TRANSACTION`) are sent one by
//! one, since the driver doesn't use the multi-statement protocol.
//!
//! # Sharing
//!
//! A `mysql::Conn` runs one statement at a time through `&mut`, so the
//! executor holds it behind a mutex, taken for each statement. That makes a
//! single executor, and a transaction on it, shareable by reference; prefer
//! [`MySqlPool`] when several threads query concurrently.

#![warn(missing_docs)]

#[cfg(feature = "dynamic")]
mod driver;
mod error;
#[cfg(feature = "migrations")]
mod migrations;
#[cfg(feature = "pool")]
pub mod pool;
mod value;

use std::sync::{Mutex, MutexGuard, PoisonError};

use mysql::prelude::Queryable;
use rupa_core::MySql;
use rupa_core::exec::{Engine, Executor, Immediate, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Transactional, Tx, TxConnection, TxOptions};

#[cfg(feature = "dynamic")]
pub use driver::MySqlDriver;
pub use error::MySqlError;
#[cfg(feature = "pool")]
pub use pool::{MySqlConn, MySqlManager, MySqlPool, MySqlPoolTx};
use value::{MyRows, to_mysql};

/// A transaction (or savepoint) on a [`MySqlExecutor`]. Rolled back when
/// dropped without `commit`.
pub type MySqlTx<'t> = Tx<&'t MySqlExecutor>;

/// Adds the connection setting RUPA relies on (`CLIENT_FOUND_ROWS`; see the
/// crate docs) to `opts`.
pub fn found_rows(opts: impl Into<mysql::Opts>) -> mysql::OptsBuilder {
    let opts: mysql::Opts = opts.into();
    let flags =
        opts.get_additional_capabilities() | mysql::consts::CapabilityFlags::CLIENT_FOUND_ROWS;
    mysql::OptsBuilder::from_opts(opts).additional_capabilities(flags)
}

/// Executes queries on a `mysql::Conn`, one statement at a time.
pub struct MySqlExecutor {
    conn: Mutex<mysql::Conn>,
    dialect: MySql,
    pending: PendingRollback,
    #[cfg(feature = "migrations")]
    migration: rupa_migrate::protocol::mysql::State,
}

impl MySqlExecutor {
    /// Wraps a connection opened with `CLIENT_FOUND_ROWS` (see [`found_rows`]).
    pub fn new(conn: mysql::Conn) -> Self {
        Self {
            conn: Mutex::new(conn),
            dialect: MySql,
            pending: PendingRollback::default(),
            #[cfg(feature = "migrations")]
            migration: rupa_migrate::protocol::mysql::State::default(),
        }
    }

    /// Connects with `opts` plus the settings RUPA relies on.
    pub fn connect(opts: impl Into<mysql::Opts>) -> Result<Self, MySqlError> {
        Ok(Self::new(mysql::Conn::new(found_rows(opts))?))
    }

    /// The underlying connection. Statements sent through it directly bypass
    /// the pending-rollback check; call [`clean`](TxConnection::clean) first.
    pub fn connection(&mut self) -> &mut mysql::Conn {
        self.conn.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// The underlying connection.
    pub fn into_inner(self) -> mysql::Conn {
        self.conn
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs a `;`-separated script statement by statement (no
    /// multi-statement protocol). For DDL and fixtures; values are not bound.
    pub fn script(&self, sql: &str) -> Result<(), MySqlError> {
        script(&mut self.lock(), sql)
    }

    /// The connection, for one statement. A statement that panicked leaves
    /// no state behind the lock (a transaction it was in rolls back when
    /// unwinding drops it), so a poisoned lock is still usable.
    fn lock(&self) -> MutexGuard<'_, mysql::Conn> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends the rollback a dropped transaction owes, on the locked `conn`.
    fn clean_on(&self, conn: &mut mysql::Conn) -> Result<(), MySqlError> {
        if let Some(statement) = self.pending.statement() {
            let sql = rupa_sql::render_tx(&statement, &self.dialect)?;

            script(conn, &sql)?;
            self.pending.clear();
        }

        Ok(())
    }

    /// Rendered SQL is cached by statement shape; the `mysql` crate caches
    /// server-side prepared statements by SQL text (`statement_cache_size`).
    ///
    /// `exec*` prepares through the connection's statement cache.
    ///
    /// Dropping the iterator drains the remaining rows.
    fn dispatch(&self, statement: &Statement, expect: Expect) -> Result<Outcome, MySqlError> {
        let mut conn = self.lock();

        self.clean_on(&mut conn)?;

        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let params = if rendered.params.is_empty() {
            mysql::Params::Empty
        } else {
            mysql::Params::Positional(
                rendered
                    .params
                    .iter()
                    .map(to_mysql)
                    .collect::<Result<_, _>>()?,
            )
        };
        if expect == Expect::Affected {
            conn.exec_drop(&*rendered.sql, params)?;
            return Ok(Outcome::Affected(conn.affected_rows()));
        }

        let limit = expect.fetch_limit().unwrap_or(usize::MAX);
        let mut rows = Vec::new();
        for row in conn.exec_iter(&*rendered.sql, params)? {
            if rows.len() == limit {
                break;
            }

            rows.push(row?.unwrap());
        }

        Ok(Outcome::Rows(Box::new(MyRows::new(rows))))
    }
}

/// Runs a `;`-separated script on `conn`, statement by statement.
fn script(conn: &mut mysql::Conn, sql: &str) -> Result<(), MySqlError> {
    for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        conn.query_drop(statement)?;
    }

    Ok(())
}

/// The connection id is left out while a statement holds the connection.
impl std::fmt::Debug for MySqlExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let connection_id = self.conn.try_lock().ok().map(|conn| conn.connection_id());

        f.debug_struct("MySqlExecutor")
            .field("connection_id", &connection_id)
            .field("dirty", &self.pending.is_pending())
            .finish_non_exhaustive()
    }
}

impl Engine for MySqlExecutor {
    type Completion = Immediate;
}

impl Executor for MySqlExecutor {
    type Dialect = MySql;
    type Error = MySqlError;

    fn dialect(&self) -> &MySql {
        &self.dialect
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, MySqlError> {
        #[cfg(feature = "migrations")]
        self.migration.outside_run()?;

        self.dispatch(statement, expect)
    }
}

impl TxConnection for MySqlExecutor {
    fn control(&self, statement: TxStatement) -> Result<(), MySqlError> {
        let mut conn = self.lock();

        self.clean_on(&mut conn)?;

        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;

        script(&mut conn, &sql)
    }

    fn defer_rollback(&self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&self) -> Result<(), MySqlError> {
        if !self.pending.is_pending() {
            return Ok(());
        }

        self.clean_on(&mut self.lock())
    }
}

impl Transactional for MySqlExecutor {
    type Tx<'t> = MySqlTx<'t>;

    fn begin_with(&self, options: TxOptions) -> Result<MySqlTx<'_>, MySqlError> {
        Tx::begin(self, options)
    }
}
