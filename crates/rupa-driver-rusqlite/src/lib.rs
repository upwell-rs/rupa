//! Synchronous RUPA executor on [`rusqlite`] (SQLite 3.38+).
//!
//! - [`SqliteExecutor`] wraps one `rusqlite::Connection`.
//! - With the `pool` feature, [`SqlitePool`] pools them (on `r2d2`) and
//!   [`SqliteDriver`] registers the driver for `sqlite:` URLs (see
//!   `rupa_core::registry`).
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
//! [`SqliteExecutor`] is a [`TxConnection`], so it is
//! [`Transactional`](rupa_core::tx::Transactional): `begin` returns a
//! [`SqliteTx`]; nested `begin` opens a savepoint. A dropped `SqliteTx`
//! rolls back immediately. If that fails, the executor is marked dirty and
//! the rollback is retried before the next statement. Read-only transactions
//! are refused: SQLite cannot enforce them.

#![warn(missing_docs)]

mod error;
#[cfg(feature = "pool")]
pub mod pool;
mod value;

use rupa_core::Sqlite;
use rupa_core::exec::{Executor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{PendingRollback, Tx, TxConnection};
use rusqlite::types::Value as SqlValue;

pub use error::SqliteError;
#[cfg(feature = "pool")]
pub use pool::{
    OpenMode, SqliteConn, SqliteDriver, SqliteManager, SqlitePool, SqlitePoolTx, SqliteTarget,
};
use value::{SqliteRows, to_sql};

/// Prepared statements cached per connection.
pub const STATEMENT_CACHE_CAPACITY: usize = 128;

/// A transaction (or savepoint) on a [`SqliteExecutor`]. Rolled back when
/// dropped without `commit`.
pub type SqliteTx<'t> = Tx<&'t mut SqliteExecutor>;

/// Executes queries on a `rusqlite::Connection`.
pub struct SqliteExecutor {
    conn: rusqlite::Connection,
    dialect: Sqlite,
    pending: PendingRollback,
}

impl SqliteExecutor {
    /// Wraps a connection, enabling Postgres-compatible `LIKE` semantics.
    pub fn new(conn: rusqlite::Connection) -> Result<Self, SqliteError> {
        conn.pragma_update(None, "case_sensitive_like", true)?;
        conn.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);
        Ok(Self {
            conn,
            dialect: Sqlite,
            pending: PendingRollback::default(),
        })
    }

    /// Keeps at most `capacity` prepared statements (`0`: prepare every
    /// query anew).
    pub fn set_statement_cache_size(&mut self, capacity: usize) {
        self.conn.set_prepared_statement_cache_capacity(capacity);
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
        &mut self.conn
    }

    /// The underlying connection.
    pub fn into_inner(self) -> rusqlite::Connection {
        self.conn
    }

    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        self.clean()?;
        // Rendered SQL is cached by statement shape, prepared statements by
        // SQL text: a repeated query skips both rendering and parsing.
        let rendered = rupa_sql::render_cached(statement, &self.dialect)?;
        let params = rusqlite::params_from_iter(rendered.params.iter().map(to_sql));
        let mut stmt = self.conn.prepare_cached(&rendered.sql)?;
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
}

impl std::fmt::Debug for SqliteExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteExecutor")
            .field("path", &self.conn.path())
            .field("dirty", &self.pending.is_pending())
            .finish_non_exhaustive()
    }
}

impl Executor for SqliteExecutor {
    type Dialect = Sqlite;
    type Error = SqliteError;

    fn dialect(&self) -> &Sqlite {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        self.run(statement, expect)
    }
}

impl TxConnection for SqliteExecutor {
    fn control(&mut self, statement: TxStatement) -> Result<(), SqliteError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.conn.execute_batch(&sql)?)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.pending.defer(depth);
    }

    fn is_dirty(&self) -> bool {
        self.pending.is_pending()
    }

    fn clean(&mut self) -> Result<(), SqliteError> {
        if let Some(statement) = self.pending.statement() {
            self.conn
                .execute_batch(&rupa_sql::render_tx(&statement, &self.dialect)?)?;
            self.pending.clear();
        }
        Ok(())
    }
}
