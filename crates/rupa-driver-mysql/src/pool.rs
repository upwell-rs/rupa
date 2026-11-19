//! Connection pooling on `r2d2`, and the registry driver for `mysql:` URLs.
//!
//! URLs are anything `mysql::Opts::from_url` parses, e.g.
//! `mysql://user:password@host:3306/db`. Connections always get
//! `CLIENT_FOUND_ROWS` (see the crate docs).
//!
//! A connection is discarded when it owes a rollback that cannot be sent;
//! `idle_timeout` and `max_lifetime` are enforced by r2d2.

use std::fmt;

use rupa_core::MySql;
use rupa_core::config::{DbConfig, MySqlOptions, PoolConfig};
use rupa_core::error::{ConnectError, DynError};
use rupa_core::exec::Immediate;
use rupa_core::pool::r2d2 as glue;
use rupa_core::pool::{Connect, OwnedTx, Pooled, SyncConnect};
use rupa_core::tx::TxConnection;

use crate::{MySqlError, MySqlExecutor, found_rows};

/// A connection checked out of a [`MySqlPool`].
pub type MySqlConn = Pooled<r2d2::PooledConnection<MySqlManager>>;

/// A transaction that owns its pooled connection (what `begin` on the pool returns).
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
        holds_migration_state(conn) || (conn.is_dirty() && conn.clean().is_err())
    }
}

rupa_core::r2d2_pool! {
    /// A pool of MySQL connections; a [`Pool`](rupa_core::pool::Pool), so repository views run sync
    /// `&self` repositories and `pool.begin()?` opens an owned transaction.
    /// Cloning shares the pool.
    pub struct MySqlPool {
        manager: MySqlManager,
        dialect: MySql,
        error: MySqlError,
    }
}

impl MySqlPool {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts,
    /// opening the first connection (so bad credentials fail here).
    pub fn new(manager: MySqlManager, config: &PoolConfig) -> Result<Self, MySqlError> {
        Self::build(manager, glue::builder(config))
    }

    /// Builds a pool for a `mysql:` URL with the config's [`MySqlOptions`].
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        Self::new(manager(config)?, &config.pool)
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }
}

impl Connect for MySqlPool {
    type Completion = Immediate;
    type Connection = MySqlExecutor;
}

impl SyncConnect for MySqlPool {
    fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        Self::connect(config)
    }

    fn connect_one(config: &DbConfig) -> Result<MySqlExecutor, ConnectError> {
        r2d2::ManageConnection::connect(&manager(config)?)
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }
}

/// The connection manager `config` describes.
fn manager(config: &DbConfig) -> Result<MySqlManager, ConnectError> {
    let opts =
        mysql::Opts::from_url(&config.url).map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;

    Ok(MySqlManager::with_options(opts, config.driver.mysql()?))
}

/// Whether a migration run left state on `conn`, so the pool must discard it
/// instead of handing it out again.
fn holds_migration_state(conn: &MySqlExecutor) -> bool {
    #[cfg(feature = "migrations")]
    let active = conn.migration.is_active();
    #[cfg(not(feature = "migrations"))]
    let active = {
        let _ = conn;

        false
    };

    active
}
