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
//! [`MySqlExecutor`] is a [`TxConnection`]: `begin` returns a [`MySqlTx`];
//! nested `begin` opens a savepoint; a dropped `MySqlTx` rolls back
//! immediately (or, if that fails, before the next statement).
//! Transaction-control statements that render as several statements (an
//! isolation level plus `START TRANSACTION`) are sent one by one, since the
//! driver doesn't use the multi-statement protocol.

#![warn(missing_docs)]

#[cfg(feature = "dynamic")]
mod driver;
mod error;
#[cfg(feature = "migrations")]
mod migrations;
#[cfg(feature = "pool")]
pub mod pool;
mod value;

use mysql::prelude::Queryable;
use rupa_core::MySql;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Tx, TxConnection};

#[cfg(feature = "dynamic")]
pub use driver::MySqlDriver;
pub use error::MySqlError;
#[cfg(feature = "pool")]
pub use pool::{MySqlConn, MySqlManager, MySqlPool, MySqlPoolTx};
use value::{MyRows, to_mysql};

/// A transaction (or savepoint) on a [`MySqlExecutor`]. Rolled back when
/// dropped without `commit`.
pub type MySqlTx<'t> = Tx<&'t mut MySqlExecutor>;

/// Adds the connection setting RUPA relies on (`CLIENT_FOUND_ROWS`; see the
/// crate docs) to `opts`.
pub fn found_rows(opts: impl Into<mysql::Opts>) -> mysql::OptsBuilder {
    let opts: mysql::Opts = opts.into();
    let flags =
        opts.get_additional_capabilities() | mysql::consts::CapabilityFlags::CLIENT_FOUND_ROWS;
    mysql::OptsBuilder::from_opts(opts).additional_capabilities(flags)
}

/// Executes queries on a `mysql::Conn`.
pub struct MySqlExecutor {
    conn: mysql::Conn,
    dialect: MySql,
    pending: PendingRollback,
    #[cfg(feature = "migrations")]
    migration: rupa_migrate::protocol::mysql::State,
}

impl MySqlExecutor {
    /// Wraps a connection opened with `CLIENT_FOUND_ROWS` (see [`found_rows`]).
    pub fn new(conn: mysql::Conn) -> Self {
        Self {
            conn,
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
        &mut self.conn
    }

    /// The underlying connection.
    pub fn into_inner(self) -> mysql::Conn {
        self.conn
    }

    /// Runs a `;`-separated script statement by statement (no
    /// multi-statement protocol). For DDL and fixtures; values are not bound.
    pub fn script(&mut self, sql: &str) -> Result<(), MySqlError> {
        for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            self.conn.query_drop(statement)?;
        }

        Ok(())
    }

    /// Rendered SQL is cached by statement shape; the `mysql` crate caches
    /// server-side prepared statements by SQL text (`statement_cache_size`).
    ///
    /// `exec*` prepares through the connection's statement cache.
    ///
    /// Dropping the iterator drains the remaining rows.
    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, MySqlError> {
        self.clean()?;
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
            self.conn.exec_drop(&*rendered.sql, params)?;
            return Ok(Outcome::Affected(self.conn.affected_rows()));
        }

        let limit = expect.fetch_limit().unwrap_or(usize::MAX);
        let mut rows = Vec::new();
        for row in self.conn.exec_iter(&*rendered.sql, params)? {
            if rows.len() == limit {
                break;
            }

            rows.push(row?.unwrap());
        }

        Ok(Outcome::Rows(Box::new(MyRows::new(rows))))
    }
}

impl std::fmt::Debug for MySqlExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySqlExecutor")
            .field("connection_id", &self.conn.connection_id())
            .field("dirty", &self.pending.is_pending())
            .finish_non_exhaustive()
    }
}

impl Executor for MySqlExecutor {
    type Dialect = MySql;
    type Error = MySqlError;

    fn dialect(&self) -> &MySql {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, MySqlError> {
        #[cfg(feature = "migrations")]
        if self.migration.is_running() {
            return self.execute_migration_statement(statement, expect);
        }

        self.run(statement, expect)
    }
}

impl TxConnection for MySqlExecutor {
    fn control(&mut self, statement: TxStatement) -> Result<(), MySqlError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        self.script(&sql)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&mut self) -> Result<(), MySqlError> {
        if let Some(statement) = self.pending.statement() {
            let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
            self.script(&sql)?;
            self.pending.clear();
        }

        Ok(())
    }
}
