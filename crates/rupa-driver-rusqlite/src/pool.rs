//! Connection pooling on `r2d2`, and the registry driver for `sqlite:` URLs.
//!
//! # URLs
//!
//! | URL | database |
//! |---|---|
//! | `sqlite::memory:`, `sqlite://:memory:` | a private in-memory database (the pool keeps exactly one connection) |
//! | `sqlite://data/app.db`, `sqlite:data/app.db` | a file, relative to the working directory |
//! | `sqlite:///var/lib/app.db`, `sqlite://C:/data/app.db` | an absolute path |
//!
//! A `?mode=` query parameter sets how a file is opened: `rwc` (read-write,
//! create if missing; the default), `rw`, or `ro`.
//!
//! # Options
//!
//! [`SqliteOptions`] (`driver = "sqlite"` in a `DbConfig`): `busy_timeout`,
//! `foreign_keys` (on by default, as references are always enforced in
//! Postgres), `journal_mode` and `statement_cache_size`.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
mod tests;

use rupa_core::Sqlite;
use rupa_core::config::{DbConfig, PoolConfig, SqliteOptions};
pub use rupa_core::config::{OpenMode, SqliteTarget};
use rupa_core::error::{ConnectError, DynError};
use rupa_core::pool::r2d2 as glue;
use rupa_core::pool::{OwnedTx, Pooled};
use rupa_core::tx::TxConnection;

use rusqlite::OpenFlags;

use crate::{SqliteError, SqliteExecutor};

/// A connection checked out of a [`SqlitePool`].
pub type SqliteConn = Pooled<r2d2::PooledConnection<SqliteManager>>;

/// A transaction that owns its pooled connection (see [`Pool::begin`](rupa_core::pool::Pool::begin)).
pub type SqlitePoolTx = OwnedTx<SqliteConn>;

type InitFn = dyn Fn(&mut rusqlite::Connection) -> rusqlite::Result<()> + Send + Sync;

/// Opens [`SqliteExecutor`]s for `r2d2`.
///
/// Every connection gets the busy timeout and pragmas configured here, then
/// the optional [`on_connect`](Self::on_connect) hook. On return to the
/// pool, a connection with a deferred rollback is cleaned, or discarded if
/// that fails.
#[derive(Clone)]
pub struct SqliteManager {
    target: SqliteTarget,
    busy_timeout: Option<Duration>,
    foreign_keys: bool,
    journal_mode: Option<String>,
    statement_cache_size: Option<usize>,
    on_connect: Option<Arc<InitFn>>,
}

impl SqliteManager {
    /// A manager for `target`, with default [`SqliteOptions`].
    pub fn new(target: SqliteTarget) -> Self {
        Self::with_options(target, SqliteOptions::default())
    }

    /// A manager for `target` with `options`.
    pub fn with_options(target: SqliteTarget, options: SqliteOptions) -> Self {
        Self {
            target,
            busy_timeout: options.busy_timeout,
            foreign_keys: options.foreign_keys,
            journal_mode: options.journal_mode,
            statement_cache_size: options.statement_cache_size,
            on_connect: None,
        }
    }

    /// Sets `busy_timeout` (`None`: fail at once on a locked database).
    pub fn busy_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.busy_timeout = timeout;
        self
    }

    /// Sets `PRAGMA foreign_keys`.
    pub fn foreign_keys(mut self, on: bool) -> Self {
        self.foreign_keys = on;
        self
    }

    /// Sets `PRAGMA journal_mode` (e.g. `"wal"`).
    pub fn journal_mode(mut self, mode: impl Into<String>) -> Self {
        self.journal_mode = Some(mode.into());
        self
    }

    /// Runs `f` on every new connection, e.g. to attach databases or load
    /// extensions.
    pub fn on_connect(
        mut self,
        f: impl Fn(&mut rusqlite::Connection) -> rusqlite::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.on_connect = Some(Arc::new(f));
        self
    }

    /// The database it connects to.
    pub fn target(&self) -> &SqliteTarget {
        &self.target
    }

    /// `journal_mode` returns the resulting mode as a row.
    fn open(&self) -> Result<rusqlite::Connection, SqliteError> {
        let mut conn = match &self.target {
            SqliteTarget::Memory => rusqlite::Connection::open_in_memory()?,
            SqliteTarget::File { path, mode } => {
                let flags = match mode {
                    OpenMode::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY,
                    OpenMode::ReadWrite => OpenFlags::SQLITE_OPEN_READ_WRITE,
                    OpenMode::ReadWriteCreate => {
                        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
                    }
                } | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX;
                rusqlite::Connection::open_with_flags(path, flags)?
            }
        };
        conn.busy_timeout(self.busy_timeout.unwrap_or_default())?;
        conn.pragma_update(None, "foreign_keys", self.foreign_keys)?;
        if let Some(mode) = &self.journal_mode {
            conn.pragma_update_and_check(None, "journal_mode", mode, |_| Ok(()))?;
        }

        if let Some(init) = &self.on_connect {
            init(&mut conn)?;
        }

        Ok(conn)
    }
}

impl fmt::Debug for SqliteManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqliteManager")
            .field("target", &self.target)
            .field("busy_timeout", &self.busy_timeout)
            .field("foreign_keys", &self.foreign_keys)
            .field("journal_mode", &self.journal_mode)
            .field("statement_cache_size", &self.statement_cache_size)
            .field("on_connect", &self.on_connect.is_some())
            .finish()
    }
}

impl r2d2::ManageConnection for SqliteManager {
    type Connection = SqliteExecutor;
    type Error = SqliteError;

    fn connect(&self) -> Result<SqliteExecutor, SqliteError> {
        let mut conn = SqliteExecutor::new(self.open()?)?;
        if let Some(size) = self.statement_cache_size {
            conn.set_statement_cache_size(size);
        }

        Ok(conn)
    }

    /// A local file can't drop the connection; just settle any debt.
    fn is_valid(&self, conn: &mut SqliteExecutor) -> Result<(), SqliteError> {
        conn.clean()
    }

    fn has_broken(&self, conn: &mut SqliteExecutor) -> bool {
        holds_migration_state(conn) || (conn.is_dirty() && conn.clean().is_err())
    }
}

rupa_core::r2d2_pool! {
    /// A pool of SQLite connections.
    ///
    /// A sync [`Pool`](rupa_core::pool::Pool): repository views run sync `&self` methods, and
    /// `pool.begin()` opens an owned transaction. Cloning shares the pool.
    pub struct SqlitePool {
        manager: SqliteManager,
        dialect: Sqlite,
        error: SqliteError,
    }
}

impl SqlitePool {
    /// Builds a pool from `manager` with `config`'s sizing, opening the
    /// first connection eagerly (so a bad path fails here).
    ///
    /// An in-memory database is private to its connection, so for
    /// [`SqliteTarget::Memory`] the pool keeps exactly one connection open
    /// and never recycles it, whatever `config` says.
    pub fn new(manager: SqliteManager, config: &PoolConfig) -> Result<Self, SqliteError> {
        let builder = glue::builder(config);
        let builder = match manager.target {
            SqliteTarget::Memory => builder
                .max_size(1)
                .min_idle(Some(1))
                .idle_timeout(None)
                .max_lifetime(None),
            SqliteTarget::File { .. } => builder,
        };

        Self::build(manager, builder)
    }

    /// Builds a pool for a `sqlite:` URL with the config's [`SqliteOptions`].
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let target = SqliteTarget::parse(&config.url).map_err(ConnectError::InvalidUrl)?;
        let manager = SqliteManager::with_options(target, config.driver.sqlite()?);
        Self::new(manager, &config.pool).map_err(|e| ConnectError::Connect(DynError::new(e)))
    }
}

/// Whether a migration run left state on `conn`, so the pool must discard it
/// instead of handing it out again.
fn holds_migration_state(conn: &SqliteExecutor) -> bool {
    #[cfg(feature = "migrations")]
    let active = conn.migration.is_active();
    #[cfg(not(feature = "migrations"))]
    let active = {
        let _ = conn;

        false
    };

    active
}
