//! Async RUPA executors on [`sqlx`]: Postgres, MySQL and SQLite.
//!
//! One generic executor, pool and registry driver, instantiated per
//! database by cargo feature:
//!
//! | feature | database | module | registry name | URL schemes |
//! |---|---|---|---|---|
//! | `postgres` | [`sqlx::Postgres`] | [`postgres`] | `sqlx-postgres` | `postgres`, `postgresql` |
//! | `mysql` | [`sqlx::MySql`] | [`mysql`] | `sqlx-mysql` | `mysql` |
//! | `sqlite` | [`sqlx::Sqlite`] | [`sqlite`] | `sqlx-sqlite` | `sqlite` |
//!
//! - [`SqlxExecutor`] wraps one sqlx connection (`PgConnection`,
//!   `MySqlConnection`, `SqliteConnection`).
//! - With the `pool` feature, [`SqlxPool`] pools them (on `deadpool`, like
//!   the tokio-postgres driver; see [`pool`] for why not sqlx's own pool).
//! - With the `dynamic` feature, `SqlxDriver` registers them (see
//!   `rupa_core::registry`).
//! - `tls-rustls` enables TLS on rustls, configured in the URL
//!   (`?sslmode=require`, `?ssl-mode=REQUIRED`).
//!
//! Every backend passes the shared conformance suite: results mean what they
//! mean on Postgres (MySQL `UPDATE`s report matched rows; sqlx sets
//! `CLIENT_FOUND_ROWS`), and values are stored exactly as the other rupa
//! drivers store them, so a database written by `rupa-driver-rusqlite` or
//! `rupa-driver-mysql` reads the same here.
//!
//! # Performance
//!
//! Two caches, so a repeated query neither renders nor parses again:
//!
//! - rendered SQL, by statement shape, process-wide (`rupa_sql::cache`);
//! - prepared statements, by SQL text, per connection: sqlx's own LRU,
//!   sized by `statement_cache_size` in the database's options (sqlx's
//!   default: 100). The SQL is handed to sqlx as the render cache's
//!   `Arc<str>`, without copying.
//!
//! # Transactions
//!
//! [`SqlxExecutor`] is an [`AsyncTxConnection`], so it is
//! [`AsyncTransactional`](rupa_core::tx::AsyncTransactional): `begin`
//! returns a [`SqlxTx`], and `begin` on that opens a savepoint. Transaction
//! control is rendered by rupa (isolation levels, read-only, savepoints) and
//! sent as plain SQL; sqlx's own `Transaction` type is not involved. A
//! `SqlxTx` dropped while open marks the executor dirty, and the rollback is
//! sent before the connection's next statement; [`SqlxPool`] also cleans
//! dirty connections when they are returned.

#![warn(missing_docs)]
// With no database feature there is nothing to instantiate the glue for.
#![cfg_attr(
    not(any(feature = "postgres", feature = "mysql", feature = "_sqlite")),
    allow(unused)
)]

#[cfg(feature = "dynamic")]
mod driver;
mod error;
#[cfg(feature = "migrations")]
mod migrations;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "pool")]
pub mod pool;
#[cfg(feature = "postgres")]
pub mod postgres;
mod rows;
#[cfg(feature = "_sqlite")]
pub mod sqlite;

use std::fmt;
use std::sync::Arc;

use rupa_core::exec::{AsyncExecutor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{AsyncTx, AsyncTxConnection, PendingRollback};
use rupa_core::{Dialect, RowError, SqlType, Value};
use sqlx::{Arguments as _, Connection as _};

#[cfg(feature = "dynamic")]
pub use driver::SqlxDriver;
pub use error::SqlxError;
#[cfg(feature = "pool")]
pub use pool::{PoolBackend, SqlxConn, SqlxManager, SqlxPool, SqlxPoolTx};
use rows::SqlxRows;
/// Re-exported so applications use the same sqlx as this crate.
pub use sqlx;

type BoxDynError = Box<dyn std::error::Error + Send + Sync>;

mod sealed {
    pub trait Sealed {}
}

/// A sqlx database rupa can drive: [`sqlx::Postgres`], [`sqlx::MySql`] or
/// [`sqlx::Sqlite`], each behind its cargo feature.
///
/// The glue between rupa and sqlx for one database: its dialect, how
/// [`Value`]s are bound and read, and the few calls whose types differ per
/// database. Sealed; the methods are implementation details.
pub trait Backend: sqlx::Database + sqlx::database::HasStatementCache + sealed::Sealed {
    /// The rupa dialect statements are rendered in.
    type Dialect: Dialect + Copy + Default + Send + Sync + 'static;

    /// The engine's migration session state.
    #[cfg(feature = "migrations")]
    #[doc(hidden)]
    type Migration: migrations::Engine<Self>;

    /// The registry name of this database's [`SqlxDriver`].
    const DRIVER: &'static str;

    /// URL schemes this database's [`SqlxDriver`] accepts.
    const SCHEMES: &'static [&'static str];

    /// Appends `value` to `args`.
    #[doc(hidden)]
    fn bind(args: &mut Self::Arguments, value: &Value) -> Result<(), BoxDynError>;

    /// Reads column `index` of `row` (in range) as `want`.
    #[doc(hidden)]
    fn read(row: &Self::Row, index: usize, want: SqlType) -> Result<Value, RowError>;

    /// Runs a prepared, cached statement; returns the affected-row count.
    #[doc(hidden)]
    fn execute<'c>(
        conn: &'c mut Self::Connection,
        sql: Arc<str>,
        args: Self::Arguments,
    ) -> impl Future<Output = Result<u64, sqlx::Error>> + Send + 'c;

    /// Runs a prepared, cached statement and collects up to `limit` rows.
    #[doc(hidden)]
    fn fetch<'c>(
        conn: &'c mut Self::Connection,
        sql: Arc<str>,
        args: Self::Arguments,
        limit: Option<usize>,
    ) -> impl Future<Output = Result<Vec<Self::Row>, sqlx::Error>> + Send + 'c;

    /// Runs `;`-separated SQL without parameters or preparing (transaction
    /// control, DDL).
    #[doc(hidden)]
    fn script<'c>(
        conn: &'c mut Self::Connection,
        sql: &'c str,
    ) -> impl Future<Output = Result<(), sqlx::Error>> + Send + 'c;

    /// Sends `sql` as one simple query, unsplit.
    #[cfg(feature = "migrations")]
    #[doc(hidden)]
    fn raw<'c>(
        conn: &'c mut Self::Connection,
        sql: &'c str,
    ) -> impl Future<Output = Result<(), sqlx::Error>> + Send + 'c;
}

/// Implements [`Backend`]'s I/O methods for one database; they are the same
/// code, but sqlx's executor bounds are only nameable per database.
macro_rules! sqlx_io {
    ($db:ty) => {
        /// rupa SQL is generated with every value bound, never spliced.
        fn execute<'c>(
            conn: &'c mut <$db as sqlx::Database>::Connection,
            sql: std::sync::Arc<str>,
            args: <$db as sqlx::Database>::Arguments,
        ) -> impl Future<Output = Result<u64, sqlx::Error>> + Send + 'c {
            async move {
                let result = sqlx::query_with(sqlx::AssertSqlSafe(sql), args)
                    .execute(&mut *conn)
                    .await?;
                Ok(result.rows_affected())
            }
        }

        #[cfg(feature = "migrations")]
        fn raw<'c>(
            conn: &'c mut <$db as sqlx::Database>::Connection,
            sql: &'c str,
        ) -> impl Future<Output = Result<(), sqlx::Error>> + Send + 'c {
            async move {
                sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                    .execute(&mut *conn)
                    .await
                    .map(drop)
            }
        }

        /// Dropping the stream discards the remaining rows.
        fn fetch<'c>(
            conn: &'c mut <$db as sqlx::Database>::Connection,
            sql: std::sync::Arc<str>,
            args: <$db as sqlx::Database>::Arguments,
            limit: Option<usize>,
        ) -> impl Future<Output = Result<Vec<<$db as sqlx::Database>::Row>, sqlx::Error>> + Send + 'c
        {
            use futures_util::{StreamExt as _, TryStreamExt as _};
            async move {
                let query = sqlx::query_with(sqlx::AssertSqlSafe(sql), args);
                match limit {
                    None => query.fetch_all(&mut *conn).await,
                    Some(limit) => query.fetch(&mut *conn).take(limit).try_collect().await,
                }
            }
        }
    };
}
pub(crate) use sqlx_io;

/// A transaction (or savepoint) on a [`SqlxExecutor`]. Dropped while open, it
/// leaves the rollback to the executor (see the crate docs).
pub type SqlxTx<'t, DB> = AsyncTx<&'t mut SqlxExecutor<DB>>;

/// Executes queries on one sqlx connection.
pub struct SqlxExecutor<DB: Backend> {
    conn: DB::Connection,
    dialect: DB::Dialect,
    pending: PendingRollback,
    broken: bool,
    #[cfg(feature = "migrations")]
    migration: DB::Migration,
}

impl<DB: Backend> SqlxExecutor<DB> {
    /// Wraps a connection.
    pub fn new(conn: DB::Connection) -> Self {
        Self {
            conn,
            dialect: DB::Dialect::default(),
            pending: PendingRollback::default(),
            broken: false,
            #[cfg(feature = "migrations")]
            migration: DB::Migration::default(),
        }
    }

    /// Connects to `url` with sqlx's defaults.
    pub async fn connect(url: &str) -> Result<Self, SqlxError> {
        Ok(Self::new(DB::Connection::connect(url).await?))
    }

    /// The underlying connection. Statements sent through it directly bypass
    /// the pending-rollback check; call [`clean`](AsyncTxConnection::clean)
    /// first.
    pub fn connection(&mut self) -> &mut DB::Connection {
        &mut self.conn
    }

    /// The underlying connection.
    pub fn into_inner(self) -> DB::Connection {
        self.conn
    }

    /// The number of prepared statements sqlx keeps for this connection.
    pub fn cached_statements(&self) -> usize {
        self.conn.cached_statements_size()
    }

    /// Whether a statement failed in a way that leaves the connection
    /// unusable (see [`SqlxError::is_fatal`]). Pools discard such
    /// connections.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Runs a `;`-separated script without binding or preparing, e.g. DDL
    /// or fixtures. Sends an owed rollback first.
    pub async fn script(&mut self, sql: &str) -> Result<(), SqlxError> {
        self.clean().await?;
        let result = DB::script(&mut self.conn, sql).await;
        result.map_err(|e| self.failed(e))
    }

    /// Rendered SQL is cached by statement shape; sqlx caches prepared
    /// statements by SQL text: a repeated query skips both.
    async fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqlxError> {
        self.clean().await?;
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let args = arguments::<DB>(&rendered.params)?;
        let result = if expect == Expect::Affected {
            DB::execute(&mut self.conn, rendered.sql, args)
                .await
                .map(Outcome::Affected)
        } else {
            DB::fetch(&mut self.conn, rendered.sql, args, expect.fetch_limit())
                .await
                .map(|rows| Outcome::Rows(Box::new(SqlxRows::<DB>::new(rows))))
        };
        result.map_err(|e| self.failed(e))
    }

    /// Whether a migration run left state on the connection, so pools must
    /// discard it.
    #[cfg(feature = "pool")]
    fn holds_migration_state(&self) -> bool {
        #[cfg(feature = "migrations")]
        let active = migrations::Engine::is_active(&self.migration);
        #[cfg(not(feature = "migrations"))]
        let active = false;

        active
    }

    fn failed(&mut self, e: impl Into<SqlxError>) -> SqlxError {
        let e = e.into();
        self.broken |= e.is_fatal();
        e
    }
}

/// `values` bound in order.
fn arguments<DB: Backend>(values: &[Value]) -> Result<DB::Arguments, SqlxError> {
    let mut args = DB::Arguments::default();

    args.reserve(values.len(), 0);

    for value in values {
        DB::bind(&mut args, value).map_err(sqlx::Error::Encode)?;
    }

    Ok(args)
}

impl<DB: Backend> fmt::Debug for SqlxExecutor<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlxExecutor")
            .field("database", &DB::NAME)
            .field("dirty", &self.pending.is_pending())
            .field("broken", &self.broken)
            .field("cached_statements", &self.cached_statements())
            .finish_non_exhaustive()
    }
}

impl<DB: Backend> AsyncExecutor for SqlxExecutor<DB> {
    type Dialect = DB::Dialect;
    type Error = SqlxError;

    fn dialect(&self) -> &DB::Dialect {
        &self.dialect
    }

    async fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, SqlxError> {
        #[cfg(feature = "migrations")]
        if migrations::Engine::is_running(&self.migration) {
            return <DB::Migration as migrations::Engine<DB>>::user_statement(
                self, statement, expect,
            )
            .await;
        }

        self.run(statement, expect).await
    }
}

impl<DB: Backend> AsyncTxConnection for SqlxExecutor<DB> {
    async fn control(&mut self, statement: TxStatement) -> Result<(), SqlxError> {
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        self.script(&sql).await
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    async fn clean(&mut self) -> Result<(), SqlxError> {
        if let Some(statement) = self.pending.statement() {
            let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
            let result = DB::script(&mut self.conn, &sql).await;
            result.map_err(|e| self.failed(e))?;
            self.pending.clear();
        }

        Ok(())
    }
}
