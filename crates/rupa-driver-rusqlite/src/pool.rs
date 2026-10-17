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

use rupa_core::config::{DbConfig, PoolConfig, SqliteOptions};
pub use rupa_core::config::{OpenMode, SqliteTarget};
use rupa_core::erased::{BoxFuture, DynError};
use rupa_core::exec::Blocking;
use rupa_core::pool::{OwnedTx, Pool, PoolStatus, Pooled};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::tx::TxConnection;
use rupa_core::{DialectId, Sqlite};
use rusqlite::OpenFlags;

use crate::{SqliteError, SqliteExecutor};

/// A connection checked out of a [`SqlitePool`].
pub type SqliteConn = Pooled<r2d2::PooledConnection<SqliteManager>>;

/// A transaction that owns its pooled connection (see [`Pool::begin`]).
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
            // `journal_mode` returns the resulting mode as a row.
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

    fn is_valid(&self, conn: &mut SqliteExecutor) -> Result<(), SqliteError> {
        // A local file can't drop the connection; just settle any debt.
        conn.clean()
    }

    fn has_broken(&self, conn: &mut SqliteExecutor) -> bool {
        conn.is_dirty() && conn.clean().is_err()
    }
}

/// A pool of SQLite connections.
///
/// A sync [`Pool`]: `Repo<SqlitePool>` runs sync `&self` repositories, and
/// `pool.begin()` opens an owned transaction. For async repositories, wrap it
/// in [`Blocking`] (or use [`Database`] via [`SqliteDriver`]).
#[derive(Clone)]
pub struct SqlitePool {
    pool: r2d2::Pool<SqliteManager>,
    dialect: Sqlite,
}

impl SqlitePool {
    /// Builds a pool from `manager` with `config`'s sizing, opening the
    /// first connection eagerly (so a bad path fails here).
    ///
    /// An in-memory database is private to its connection, so for
    /// [`SqliteTarget::Memory`] the pool keeps exactly one connection open
    /// and never recycles it, whatever `config` says.
    pub fn new(manager: SqliteManager, config: &PoolConfig) -> Result<Self, SqliteError> {
        // Fail fast, with the driver's own error: r2d2 would retry until the
        // acquire timeout and then report only that.
        drop(r2d2::ManageConnection::connect(&manager)?);
        let builder = r2d2::Pool::builder();
        let builder = if manager.target == SqliteTarget::Memory {
            builder
                .max_size(1)
                .min_idle(Some(1))
                .idle_timeout(None)
                .max_lifetime(None)
        } else {
            builder
                .max_size(config.max_size)
                .min_idle(Some(config.min_idle.unwrap_or(1).min(config.max_size)))
                .idle_timeout(config.idle_timeout)
                .max_lifetime(config.max_lifetime)
        };
        let pool = builder
            .connection_timeout(timeout(config.acquire_timeout))
            .build(manager)
            .map_err(|e| SqliteError::Pool(Box::new(e)))?;
        Ok(Self::from_r2d2(pool))
    }

    /// Builds a pool for a `sqlite:` URL with the config's [`SqliteOptions`].
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let target = SqliteTarget::parse(&config.url).map_err(ConnectError::InvalidUrl)?;
        let manager = SqliteManager::with_options(target, config.driver.sqlite()?);
        Self::new(manager, &config.pool).map_err(|e| ConnectError::Connect(DynError::new(e)))
    }

    /// Wraps an `r2d2` pool built elsewhere.
    pub fn from_r2d2(pool: r2d2::Pool<SqliteManager>) -> Self {
        Self {
            pool,
            dialect: Sqlite,
        }
    }

    /// The underlying `r2d2` pool.
    pub fn r2d2(&self) -> &r2d2::Pool<SqliteManager> {
        &self.pool
    }
}

impl fmt::Debug for SqlitePool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlitePool")
            .field("status", &self.status())
            .finish()
    }
}

/// r2d2 has no "wait forever"; a century will do.
fn timeout(t: Option<Duration>) -> Duration {
    t.filter(|t| !t.is_zero())
        .unwrap_or(Duration::from_secs(100 * 365 * 24 * 3600))
}

impl Pool for SqlitePool {
    type Conn = SqliteConn;

    fn dialect(&self) -> &Sqlite {
        &self.dialect
    }

    fn get(&self) -> Result<SqliteConn, SqliteError> {
        self.pool
            .get()
            .map(Pooled::new)
            .map_err(|e| SqliteError::Pool(Box::new(e)))
    }

    fn status(&self) -> PoolStatus {
        let state = self.pool.state();
        PoolStatus {
            max_size: self.pool.max_size(),
            size: state.connections,
            idle: state.idle_connections,
        }
    }
}

/// The registry entry for this driver: name `sqlite`, scheme `sqlite`.
///
/// Builds a [`SqlitePool`], served to async code through [`Blocking`]
/// (statements run on the calling thread).
#[derive(Debug, Clone, Copy, Default)]
pub struct SqliteDriver;

impl Driver for SqliteDriver {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn dialect(&self) -> DialectId {
        DialectId::Sqlite
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["sqlite"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        let database = SqlitePool::connect(config).map(|pool| Database::new(Blocking(pool)));
        Box::pin(std::future::ready(database))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn urls() {
        assert_eq!(
            SqliteTarget::parse("sqlite::memory:"),
            Ok(SqliteTarget::Memory)
        );
        assert_eq!(
            SqliteTarget::parse("sqlite://:memory:"),
            Ok(SqliteTarget::Memory)
        );
        let file = |p: &str, mode| SqliteTarget::File {
            path: PathBuf::from(p),
            mode,
        };
        assert_eq!(
            SqliteTarget::parse("sqlite://data/app.db"),
            Ok(file("data/app.db", OpenMode::ReadWriteCreate))
        );
        assert_eq!(
            SqliteTarget::parse("SQLITE:app.db?mode=ro"),
            Ok(file("app.db", OpenMode::ReadOnly))
        );
        assert_eq!(
            SqliteTarget::parse("sqlite:///var/app.db?mode=rw"),
            Ok(file("/var/app.db", OpenMode::ReadWrite))
        );
        assert!(SqliteTarget::parse("sqlite://").is_err());
        assert!(SqliteTarget::parse("sqlite://x.db?cache=shared").is_err());
        assert!(SqliteTarget::parse("postgres://x").is_err());
    }
}
