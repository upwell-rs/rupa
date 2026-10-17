//! Connection pooling on `deadpool`, and the registry drivers.
//!
//! # Why not sqlx's own pool
//!
//! A pooled rupa connection carries state sqlx's pool can't hold: the
//! rollback a dropped transaction still owes (see the crate docs), which must
//! be sent before anyone else uses the connection. `deadpool` lets the pool
//! own [`SqlxExecutor`]s, so that state travels with the connection and is
//! settled when it is recycled, exactly as in the tokio-postgres driver.
//! Pool sizing and timeouts come from the same [`PoolConfig`] as every other
//! driver. sqlx still does everything below the connection: the wire
//! protocol, TLS, and the prepared-statement cache.
//!
//! The pool needs a tokio runtime, as sqlx's `runtime-tokio` does.
//!
//! # Recycling
//!
//! A connection returned to the pool is discarded if a statement failed in
//! a way that broke it (I/O, TLS, protocol), if it is older than
//! `max_lifetime`, or idle for longer than `idle_timeout`. A connection
//! whose transaction was dropped while open is rolled back before reuse, or
//! discarded if that fails.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use deadpool::managed::{self, Metrics, RecycleError, RecycleResult};
use rupa_core::config::{DbConfig, PoolConfig};
use rupa_core::erased::{BoxFuture, DynError};
use rupa_core::pool::{AsyncPool, OwnedAsyncTx, PoolStatus, Pooled};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::tx::AsyncTxConnection;
use rupa_core::{Dialect as _, DialectId};
use sqlx::ConnectOptions as _;

use crate::{Backend, SqlxError, SqlxExecutor};

/// sqlx's connection settings for `DB` (`PgConnectOptions`, ...).
pub type ConnectOptions<DB> = <<DB as sqlx::Database>::Connection as sqlx::Connection>::Options;

/// A connection checked out of a [`SqlxPool`].
pub type SqlxConn<DB> = Pooled<managed::Object<SqlxManager<DB>>>;

/// A transaction that owns its pooled connection (see [`AsyncPool::begin`]).
pub type SqlxPoolTx<DB> = OwnedAsyncTx<SqlxConn<DB>>;

/// Opens and recycles [`SqlxExecutor`]s for `deadpool`.
pub struct SqlxManager<DB: Backend> {
    options: ConnectOptions<DB>,
    connect_timeout: Option<Duration>,
    init: Arc<[String]>,
    max_connections: Option<u32>,
    idle_timeout: Option<Duration>,
    max_lifetime: Option<Duration>,
}

impl<DB: Backend> SqlxManager<DB> {
    /// Connects with sqlx's `options` (e.g. a `PgConnectOptions` parsed
    /// from a URL and adjusted).
    pub fn new(options: ConnectOptions<DB>) -> Self {
        Self {
            options,
            connect_timeout: None,
            init: Arc::new([]),
            max_connections: None,
            idle_timeout: None,
            max_lifetime: None,
        }
    }

    /// Builds the manager a [`DbConfig`] describes (URL and driver options).
    pub fn from_config(config: &DbConfig) -> Result<Self, ConnectError> {
        DB::manager(config)
    }

    /// Fails opening a connection that takes longer than `timeout`.
    pub fn connect_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Runs `statements` on every new connection, e.g. `SET` statements or
    /// `ATTACH DATABASE`.
    pub fn init(mut self, statements: impl IntoIterator<Item = String>) -> Self {
        self.init = statements.into_iter().collect();
        self
    }

    /// Caps the pool at `n` connections whatever its [`PoolConfig`] says
    /// (an in-memory SQLite database must have exactly one).
    pub fn max_connections(mut self, n: Option<u32>) -> Self {
        self.max_connections = n;
        self
    }

    /// Discards connections idle for longer than `timeout` when they are
    /// next checked out.
    pub fn idle_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Discards connections older than `lifetime` when they are next checked
    /// out.
    pub fn max_lifetime(mut self, lifetime: Option<Duration>) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    /// sqlx's connection settings.
    pub fn options(&self) -> &ConnectOptions<DB> {
        &self.options
    }

    /// Opens and initializes one connection.
    pub async fn connect(&self) -> Result<SqlxExecutor<DB>, SqlxError> {
        let connect = self.options.connect();
        let conn = match self.connect_timeout {
            None => connect.await?,
            Some(timeout) => tokio::time::timeout(timeout, connect).await.map_err(|_| {
                sqlx::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("connecting took longer than {timeout:?}"),
                ))
            })??,
        };
        let mut conn = SqlxExecutor::new(conn);
        for statement in self.init.iter() {
            conn.script(statement).await?;
        }
        Ok(conn)
    }
}

impl<DB: Backend> Clone for SqlxManager<DB> {
    fn clone(&self) -> Self {
        Self {
            options: self.options.clone(),
            connect_timeout: self.connect_timeout,
            init: self.init.clone(),
            max_connections: self.max_connections,
            idle_timeout: self.idle_timeout,
            max_lifetime: self.max_lifetime,
        }
    }
}

impl<DB: Backend> fmt::Debug for SqlxManager<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // sqlx's options redact the password.
        f.debug_struct("SqlxManager")
            .field("options", &self.options)
            .field("connect_timeout", &self.connect_timeout)
            .field("init", &self.init)
            .field("max_connections", &self.max_connections)
            .field("idle_timeout", &self.idle_timeout)
            .field("max_lifetime", &self.max_lifetime)
            .finish()
    }
}

impl<DB: Backend> managed::Manager for SqlxManager<DB> {
    type Type = SqlxExecutor<DB>;
    type Error = SqlxError;

    async fn create(&self) -> Result<SqlxExecutor<DB>, SqlxError> {
        self.connect().await
    }

    async fn recycle(
        &self,
        conn: &mut SqlxExecutor<DB>,
        metrics: &Metrics,
    ) -> RecycleResult<SqlxError> {
        if conn.is_broken() {
            return Err(RecycleError::message(
                "connection broken by an earlier error",
            ));
        }
        if self.max_lifetime.is_some_and(|max| metrics.age() > max) {
            return Err(RecycleError::message("connection reached max_lifetime"));
        }
        if self
            .idle_timeout
            .is_some_and(|max| metrics.last_used() > max)
        {
            return Err(RecycleError::message(
                "connection idle for longer than idle_timeout",
            ));
        }
        conn.clean().await.map_err(RecycleError::Backend)
    }
}

/// A pool of sqlx connections; an [`AsyncPool`], so `Repo<SqlxPool<DB>>`
/// runs `&self` repositories and `pool.begin().await?` opens an owned
/// transaction. Cloning shares the pool.
pub struct SqlxPool<DB: Backend> {
    pool: managed::Pool<SqlxManager<DB>>,
    dialect: DB::Dialect,
}

impl<DB: Backend> SqlxPool<DB> {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts, and
    /// opens the first connection (so bad credentials fail here).
    pub async fn new(manager: SqlxManager<DB>, config: &PoolConfig) -> Result<Self, SqlxError> {
        let max_size = manager
            .max_connections
            .map_or(config.max_size, |cap| cap.min(config.max_size));
        let manager = manager
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime);
        let pool = managed::Pool::builder(manager)
            .max_size(max_size as usize)
            .wait_timeout(config.acquire_timeout)
            .create_timeout(config.acquire_timeout)
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|e| SqlxError::Pool(Box::new(e)))?;
        let pool = Self::from_deadpool(pool);
        drop(pool.get().await?);
        Ok(pool)
    }

    /// Builds a pool for `config`'s URL with its driver options (see the
    /// database's module).
    pub async fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let manager = SqlxManager::from_config(config)?;
        Self::new(manager, &config.pool)
            .await
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }

    /// Wraps a `deadpool` pool built elsewhere.
    pub fn from_deadpool(pool: managed::Pool<SqlxManager<DB>>) -> Self {
        Self {
            pool,
            dialect: DB::Dialect::default(),
        }
    }

    /// The underlying `deadpool` pool.
    pub fn deadpool(&self) -> &managed::Pool<SqlxManager<DB>> {
        &self.pool
    }
}

impl<DB: Backend> Clone for SqlxPool<DB> {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            dialect: self.dialect,
        }
    }
}

impl<DB: Backend> fmt::Debug for SqlxPool<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlxPool")
            .field("manager", self.pool.manager())
            .field("status", &self.status())
            .finish()
    }
}

impl<DB: Backend> AsyncPool for SqlxPool<DB> {
    type Conn = SqlxConn<DB>;

    fn dialect(&self) -> &DB::Dialect {
        &self.dialect
    }

    async fn get(&self) -> Result<SqlxConn<DB>, SqlxError> {
        match self.pool.get().await {
            Ok(conn) => Ok(Pooled::new(conn)),
            Err(managed::PoolError::Backend(e)) => Err(e),
            Err(e) => Err(SqlxError::Pool(Box::new(e))),
        }
    }

    fn status(&self) -> PoolStatus {
        let status = self.pool.status();
        let as_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        PoolStatus {
            max_size: as_u32(status.max_size),
            size: as_u32(status.size),
            idle: as_u32(status.available),
        }
    }
}

/// The registry entry for `DB`: named `sqlx-postgres`, `sqlx-mysql` or
/// `sqlx-sqlite` (see the crate docs for the URL schemes). Builds a
/// [`SqlxPool`].
pub struct SqlxDriver<DB>(PhantomData<fn() -> DB>);

impl<DB> SqlxDriver<DB> {
    /// The driver.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<DB> Default for SqlxDriver<DB> {
    fn default() -> Self {
        Self::new()
    }
}

impl<DB> Clone for SqlxDriver<DB> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<DB> Copy for SqlxDriver<DB> {}

impl<DB: Backend> fmt::Debug for SqlxDriver<DB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SqlxDriver").field(&DB::DRIVER).finish()
    }
}

impl<DB: Backend> Driver for SqlxDriver<DB> {
    fn name(&self) -> &'static str {
        DB::DRIVER
    }

    fn dialect(&self) -> DialectId {
        DB::Dialect::default().id()
    }

    fn schemes(&self) -> &'static [&'static str] {
        DB::SCHEMES
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        Box::pin(async move { SqlxPool::<DB>::connect(config).await.map(Database::new) })
    }
}
