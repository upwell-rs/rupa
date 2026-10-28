//! Synchronous RUPA executor on [`rusqlite`] (SQLite 3.38+).
//!
//! - [`SqliteExecutor`] wraps one `rusqlite::Connection`.
//! - With the `pool` feature, [`SqlitePool`] pools them (on `r2d2`).
//! - With the `dynamic` feature, `SqliteDriver` registers the driver for
//!   `sqlite:` URLs (see `rupa_core::registry`).
//!
//! # Storage formats
//!
//! SQLite has no date, UUID or JSON types; values are stored as text in
//! fixed formats, so that text order is value order:
//!
//! | value | stored as |
//! |---|---|
//! | `bool` | `INTEGER` 0/1 |
//! | `DateTime<Utc>` | `TEXT` `YYYY-MM-DDTHH:MM:SS.ffffffZ` (UTC, always 6 digits) |
//! | `NaiveDateTime` | `TEXT` `YYYY-MM-DDTHH:MM:SS.ffffff` |
//! | `NaiveDate` / `NaiveTime` | `TEXT` `YYYY-MM-DD` / `HH:MM:SS.ffffff` |
//! | `Uuid` | `TEXT`, hyphenated |
//! | JSON | `TEXT` |
//!
//! Reading also accepts other common forms (space separator, offsets,
//! 16-byte UUID blobs).
//!
//! # Semantics
//!
//! [`SqliteExecutor::new`] enables `PRAGMA case_sensitive_like`, so `LIKE`
//! is case-sensitive as in Postgres.
//!
//! # Performance
//!
//! Two caches, so a repeated query neither renders nor parses again:
//!
//! - rendered SQL, by statement shape, process-wide (`rupa_sql::cache`);
//! - prepared statements, by SQL text, per connection (rusqlite's
//!   `prepare_cached`, an LRU of [`STATEMENT_CACHE_CAPACITY`] statements by
//!   default; see [`SqliteExecutor::set_statement_cache_size`]).
//!
//! # Transactions
//!
//! [`SqliteExecutor`] is a [`TxConnection`] and
//! [`Transactional`]: `begin` returns a [`SqliteTx`]; nested `begin` opens a
//! savepoint. A dropped `SqliteTx` rolls back immediately. If that fails, the
//! executor is marked dirty and the rollback is retried before the next
//! statement. Read-only transactions are refused: SQLite cannot enforce them.
//!
//! # Sharing
//!
//! A `rusqlite::Connection` cannot be used from two threads at once, so the
//! executor holds it behind a mutex, taken for each statement. That makes a
//! single executor, and a transaction on it, shareable by reference; prefer
//! [`SqlitePool`] when several threads query concurrently.

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

use rupa_core::Sqlite;
use rupa_core::exec::{Engine, Executor, Immediate, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Transactional, Tx, TxConnection, TxOptions};
use rusqlite::types::Value as SqlValue;

#[cfg(feature = "dynamic")]
pub use driver::SqliteDriver;
pub use error::SqliteError;
#[cfg(feature = "pool")]
pub use pool::{OpenMode, SqliteConn, SqliteManager, SqlitePool, SqlitePoolTx, SqliteTarget};
use value::{Param, SqliteRows};

/// Prepared statements cached per connection.
pub const STATEMENT_CACHE_CAPACITY: usize = 128;

/// A transaction (or savepoint) on a [`SqliteExecutor`]. Rolled back when
/// dropped without `commit`.
pub type SqliteTx<'t> = Tx<&'t SqliteExecutor>;

/// Executes queries on a `rusqlite::Connection`, one statement at a time.
pub struct SqliteExecutor {
    conn: Mutex<rusqlite::Connection>,
    dialect: Sqlite,
    pending: PendingRollback,
    #[cfg(feature = "migrations")]
    migration: rupa_migrate::protocol::sqlite::State,
}

impl SqliteExecutor {
    /// Wraps a connection, enabling Postgres-compatible `LIKE` semantics.
    pub fn new(conn: rusqlite::Connection) -> Result<Self, SqliteError> {
        conn.pragma_update(None, "case_sensitive_like", true)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
        Ok(Self {
            conn: Mutex::new(conn),
            dialect: Sqlite,
            pending: PendingRollback::default(),
            #[cfg(feature = "migrations")]
            migration: rupa_migrate::protocol::sqlite::State::default(),
        })
    }

    /// Keeps at most `capacity` prepared statements (`0`: prepare every
    /// query anew).
    pub fn set_statement_cache_size(&mut self, capacity: usize) {
        self.connection()
            .set_prepared_statement_cache_capacity(capacity);
    }

    /// Opens a private in-memory database.
    pub fn open_in_memory() -> Result<Self, SqliteError> {
        Self::new(rusqlite::Connection::open_in_memory()?)
    }

    /// Opens (or creates) the database file at `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, SqliteError> {
        Self::new(rusqlite::Connection::open(path)?)
    }

    /// The underlying connection. Statements sent through it directly bypass
    /// the pending-rollback check; call [`clean`](TxConnection::clean) first.
    pub fn connection(&mut self) -> &mut rusqlite::Connection {
        self.conn.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// The underlying connection.
    pub fn into_inner(self) -> rusqlite::Connection {
        self.conn
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The connection, for one statement. A statement that panicked leaves
    /// no state behind the lock (a transaction it was in rolls back when
    /// unwinding drops it), so a poisoned lock is still usable.
    fn lock(&self) -> MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Rendered SQL is cached by statement shape, prepared statements by
    /// SQL text: a repeated query skips both rendering and parsing.
    fn dispatch(&self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        let conn = self.lock();

        self.clean_on(&conn)?;

        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let params = rusqlite::params_from_iter(rendered.params.iter().map(Param));
        let mut stmt = conn.prepare_cached(&rendered.sql)?;
        if expect == Expect::Affected {
            return Ok(Outcome::Affected(stmt.execute(params)? as u64));
        }

        let limit = expect.fetch_limit().unwrap_or(usize::MAX);
        let width = stmt.column_count();
        let mut rows = stmt.query(params)?;
        let mut out = Vec::new();
        while out.len() < limit {
            let Some(row) = rows.next()? else { break };
            out.push(
                (0..width)
                    .map(|i| row.get::<_, SqlValue>(i))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }

        Ok(Outcome::Rows(Box::new(SqliteRows::new(out))))
    }

    /// Sends the rollback a dropped transaction owes, on the locked `conn`.
    fn clean_on(&self, conn: &rusqlite::Connection) -> Result<(), SqliteError> {
        if let Some(statement) = self.pending.statement() {
            conn.execute_batch(&rupa_sql::render_tx(&statement, &self.dialect)?)?;
            self.pending.clear();
        }

        Ok(())
    }
}

/// The path is left out while a statement holds the connection.
impl std::fmt::Debug for SqliteExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self
            .conn
            .try_lock()
            .ok()
            .map(|conn| conn.path().map(String::from));

        f.debug_struct("SqliteExecutor")
            .field("path", &path.flatten())
            .field("dirty", &self.pending.is_pending())
            .finish_non_exhaustive()
    }
}

impl Engine for SqliteExecutor {
    type Completion = Immediate;
}

impl Executor for SqliteExecutor {
    type Dialect = Sqlite;
    type Error = SqliteError;

    fn dialect(&self) -> &Sqlite {
        &self.dialect
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        #[cfg(feature = "migrations")]
        self.migration.outside_run()?;

        self.dispatch(statement, expect)
    }
}

impl TxConnection for SqliteExecutor {
    fn control(&self, statement: TxStatement) -> Result<(), SqliteError> {
        let conn = self.lock();

        self.clean_on(&conn)?;

        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;

        Ok(conn.execute_batch(&sql)?)
    }

    fn defer_rollback(&self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&self) -> Result<(), SqliteError> {
        if !self.pending.is_pending() {
            return Ok(());
        }

        self.clean_on(&self.lock())
    }
}

impl Transactional for SqliteExecutor {
    type Tx<'t> = SqliteTx<'t>;

    fn begin_with(&self, options: TxOptions) -> Result<SqliteTx<'_>, SqliteError> {
        Tx::begin(self, options)
    }
}
