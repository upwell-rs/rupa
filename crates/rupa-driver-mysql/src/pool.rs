//! Connection pooling on `r2d2`, and the registry driver for `mysql:` URLs.
//!
//! URLs are anything `mysql::Opts::from_url` parses, e.g.
//! `mysql://user:password@host:3306/db`. Connections always get
//! `CLIENT_FOUND_ROWS` (see the crate docs).
//!
//! A connection is discarded when it owes a rollback that cannot be sent;
//! `idle_timeout` and `max_lifetime` are enforced by r2d2.

use std::fmt;
use std::time::Duration;

use rupa_core::config::{DbConfig, MySqlOptions, PoolConfig};
use rupa_core::erased::{BoxFuture, DynError};
use rupa_core::exec::Blocking;
use rupa_core::pool::{OwnedTx, Pool, PoolStatus, Pooled};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::tx::TxConnection;
use rupa_core::{DialectId, MySql};

use crate::{MySqlError, MySqlExecutor, found_rows};

/// A connection checked out of a [`MySqlPool`].
pub type MySqlConn = Pooled<r2d2::PooledConnection<MySqlManager>>;

/// A transaction that owns its pooled connection (see [`Pool::begin`]).
pub type MySqlPoolTx = OwnedTx<MySqlConn>;

/// Opens [`MySqlExecutor`]s for `r2d2`.
#[derive(Clone)]
pub struct MySqlManager {
    opts: mysql::Opts,
}

impl MySqlManager {
    /// Connects with `opts` (plus `CLIENT_FOUND_ROWS`).
    pub fn new(opts: impl Into<mysql::Opts>) -> Self {
        Self {
            opts: found_rows(opts).into(),
        }
    }

    /// Connects with `opts` adjusted by `options` (plus `CLIENT_FOUND_ROWS`).
    pub fn with_options(opts: impl Into<mysql::Opts>, options: MySqlOptions) -> Self {
        let mut builder = found_rows(opts);
        if let Some(size) = options.statement_cache_size {
            builder = builder.stmt_cache_size(size);
        }
        if !options.init.is_empty() {
            builder = builder.init(options.init);
        }
        Self {
            opts: builder.into(),
        }
    }
}

impl fmt::Debug for MySqlManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlManager")
            .field("host", &self.opts.get_ip_or_hostname())
            .field("port", &self.opts.get_tcp_port())
            .field("db", &self.opts.get_db_name())
            .field("user", &self.opts.get_user())
            .finish_non_exhaustive()
    }
}

impl r2d2::ManageConnection for MySqlManager {
    type Connection = MySqlExecutor;
    type Error = MySqlError;

    fn connect(&self) -> Result<MySqlExecutor, MySqlError> {
        Ok(MySqlExecutor::new(mysql::Conn::new(self.opts.clone())?))
    }

    fn is_valid(&self, conn: &mut MySqlExecutor) -> Result<(), MySqlError> {
        conn.clean()
    }

    fn has_broken(&self, conn: &mut MySqlExecutor) -> bool {
        conn.is_dirty() && conn.clean().is_err()
    }
}

/// A pool of MySQL connections; a [`Pool`], so `Repo<MySqlPool>` runs sync
/// `&self` repositories and `pool.begin()?` opens an owned transaction.
/// Cloning shares the pool.
#[derive(Clone)]
pub struct MySqlPool {
    pool: r2d2::Pool<MySqlManager>,
    dialect: MySql,
}

impl MySqlPool {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts,
    /// opening the first connection (so bad credentials fail here).
    pub fn new(manager: MySqlManager, config: &PoolConfig) -> Result<Self, MySqlError> {
        // Fail fast, with the driver's own error: r2d2 would retry until the
        // acquire timeout and then report only that.
        drop(r2d2::ManageConnection::connect(&manager)?);
        let pool = r2d2::Pool::builder()
            .max_size(config.max_size)
            .min_idle(Some(config.min_idle.unwrap_or(1).min(config.max_size)))
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime)
            .connection_timeout(timeout(config.acquire_timeout))
            .build(manager)
            .map_err(|e| MySqlError::Pool(Box::new(e)))?;
        Ok(Self::from_r2d2(pool))
    }

    /// Builds a pool for a `mysql:` URL with the config's [`MySqlOptions`].
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let opts = mysql::Opts::from_url(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        let manager = MySqlManager::with_options(opts, config.driver.mysql()?);
        Self::new(manager, &config.pool).map_err(|e| ConnectError::Connect(DynError::new(e)))
    }

    /// Wraps an `r2d2` pool built elsewhere.
    pub fn from_r2d2(pool: r2d2::Pool<MySqlManager>) -> Self {
        Self {
            pool,
            dialect: MySql,
        }
    }

    /// The underlying `r2d2` pool.
    pub fn r2d2(&self) -> &r2d2::Pool<MySqlManager> {
        &self.pool
    }
}

impl fmt::Debug for MySqlPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlPool")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

/// r2d2 has no "wait forever"; a century will do.
fn timeout(t: Option<Duration>) -> Duration {
    t.filter(|t| !t.is_zero())
        .unwrap_or(Duration::from_secs(100 * 365 * 24 * 3600))
}

impl Pool for MySqlPool {
    type Conn = MySqlConn;

    fn dialect(&self) -> &MySql {
        &self.dialect
    }

    fn get(&self) -> Result<MySqlConn, MySqlError> {
        self.pool
            .get()
            .map(Pooled::new)
            .map_err(|e| MySqlError::Pool(Box::new(e)))
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

/// The registry entry for this driver: name `mysql`, scheme `mysql`. Builds
/// a [`MySqlPool`], served to async code through [`Blocking`].
#[derive(Debug, Clone, Copy, Default)]
pub struct MySqlDriver;

impl Driver for MySqlDriver {
    fn name(&self) -> &'static str {
        "mysql"
    }

    fn dialect(&self) -> DialectId {
        DialectId::MySql
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["mysql"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        let database = MySqlPool::connect(config).map(|pool| Database::new(Blocking(pool)));
        Box::pin(std::future::ready(database))
    }
}
