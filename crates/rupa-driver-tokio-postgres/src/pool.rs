//! Connection pooling on `deadpool`, and the registry driver for
//! `postgres:` URLs.
//!
//! Connections are opened with `tokio_postgres::Config::connect` and their
//! connection futures are spawned on the current tokio runtime, so the pool
//! must be used inside one.
//!
//! # URLs
//!
//! Anything `tokio_postgres::Config` parses:
//! `postgres://user:password@host:5432/db?application_name=app`, or the
//! key-value form. The registry driver connects without TLS; for TLS, build a
//! [`PgManager::with_tls`] and a [`PgPool`] yourself, then
//! `Database::new(pool)`.
//!
//! # Recycling
//!
//! A connection returned to the pool is discarded if it is closed, older
//! than `max_lifetime`, or idle for longer than `idle_timeout`. A connection
//! whose transaction was dropped while open is rolled back (cleaned) before
//! reuse, or discarded if that fails.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use deadpool::managed::{self, Metrics, RecycleError, RecycleResult};
use rupa_core::config::{DbConfig, PoolConfig};
use rupa_core::erased::{BoxFuture, DynError};
use rupa_core::pool::{AsyncPool, OwnedAsyncTx, PoolStatus, Pooled};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::tx::AsyncTxConnection;
use rupa_core::{DialectId, Postgres};
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::{NoTls, Socket};

use crate::{PgError, PgExecutor};

/// A connection checked out of a [`PgPool`].
pub type PgConn = Pooled<managed::Object<PgManager>>;

/// A transaction that owns its pooled connection (see [`AsyncPool::begin`]).
pub type PgPoolTx = OwnedAsyncTx<PgConn>;

type ConnectFn = dyn Fn(
        tokio_postgres::Config,
    ) -> BoxFuture<'static, Result<tokio_postgres::Client, tokio_postgres::Error>>
    + Send
    + Sync;

/// Opens and recycles [`PgExecutor`]s for `deadpool`.
#[derive(Clone)]
pub struct PgManager {
    config: tokio_postgres::Config,
    connect: Arc<ConnectFn>,
    idle_timeout: Option<Duration>,
    max_lifetime: Option<Duration>,
}

impl PgManager {
    /// Connects without TLS.
    pub fn new(config: tokio_postgres::Config) -> Self {
        Self::with_tls(config, NoTls)
    }

    /// Connects with `tls` (e.g. `postgres-native-tls` or
    /// `tokio-postgres-rustls` connectors).
    pub fn with_tls<T>(config: tokio_postgres::Config, tls: T) -> Self
    where
        T: MakeTlsConnect<Socket> + Clone + Send + Sync + 'static,
        T::Stream: Send + 'static,
        T::TlsConnect: Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        let connect = move |config: tokio_postgres::Config| -> BoxFuture<'static, _> {
            let tls = tls.clone();
            Box::pin(async move {
                let (client, connection) = config.connect(tls).await?;
                tokio::spawn(async move {
                    // Ends when the client is dropped or the server hangs up;
                    // the client then reports itself closed.
                    let _ = connection.await;
                });
                Ok(client)
            })
        };
        Self {
            config,
            connect: Arc::new(connect),
            idle_timeout: None,
            max_lifetime: None,
        }
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

    /// The connection settings.
    pub fn config(&self) -> &tokio_postgres::Config {
        &self.config
    }
}

impl fmt::Debug for PgManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgManager")
            .field("hosts", &self.config.get_hosts())
            .field("dbname", &self.config.get_dbname())
            .field("user", &self.config.get_user())
            .field("idle_timeout", &self.idle_timeout)
            .field("max_lifetime", &self.max_lifetime)
            .finish_non_exhaustive()
    }
}

impl managed::Manager for PgManager {
    type Type = PgExecutor;
    type Error = PgError;

    async fn create(&self) -> Result<PgExecutor, PgError> {
        let client = (self.connect)(self.config.clone()).await?;
        Ok(PgExecutor::new(client))
    }

    async fn recycle(&self, conn: &mut PgExecutor, metrics: &Metrics) -> RecycleResult<PgError> {
        if conn.client().is_closed() {
            return Err(RecycleError::message("connection closed"));
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

/// A pool of Postgres connections; an [`AsyncPool`], so `Repo<PgPool>` runs
/// `&self` repositories and `pool.begin().await?` opens an owned
/// transaction. Cloning shares the pool.
#[derive(Clone)]
pub struct PgPool {
    pool: managed::Pool<PgManager>,
    dialect: Postgres,
}

impl PgPool {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts, and
    /// opens the first connection (so bad credentials fail here).
    pub async fn new(manager: PgManager, config: &PoolConfig) -> Result<Self, PgError> {
        let manager = manager
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime);
        let pool = managed::Pool::builder(manager)
            .max_size(config.max_size as usize)
            .wait_timeout(config.acquire_timeout)
            .create_timeout(config.acquire_timeout)
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|e| PgError::Pool(Box::new(e)))?;
        let pool = Self::from_deadpool(pool);
        drop(pool.get().await?);
        Ok(pool)
    }

    /// Builds a pool for a `postgres:` URL with the config's
    /// [`PostgresOptions`](rupa_core::config::PostgresOptions), without TLS.
    pub async fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let mut pg = tokio_postgres::Config::from_str(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        let options = config.driver.postgres()?;
        if let Some(name) = &options.application_name {
            pg.application_name(name);
        }
        if let Some(timeout) = options.connect_timeout {
            pg.connect_timeout(timeout);
        }
        Self::new(PgManager::new(pg), &config.pool)
            .await
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }

    /// Wraps a `deadpool` pool built elsewhere.
    pub fn from_deadpool(pool: managed::Pool<PgManager>) -> Self {
        Self {
            pool,
            dialect: Postgres,
        }
    }

    /// The underlying `deadpool` pool.
    pub fn deadpool(&self) -> &managed::Pool<PgManager> {
        &self.pool
    }
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgPool")
            .field("manager", self.pool.manager())
            .field("status", &self.status())
            .finish()
    }
}

impl AsyncPool for PgPool {
    type Conn = PgConn;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    async fn get(&self) -> Result<PgConn, PgError> {
        match self.pool.get().await {
            Ok(conn) => Ok(Pooled::new(conn)),
            Err(managed::PoolError::Backend(e)) => Err(e),
            Err(e) => Err(PgError::Pool(Box::new(e))),
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

/// The registry entry for this driver: name `postgres`, schemes
/// `postgres` and `postgresql`. Builds a [`PgPool`] without TLS.
#[derive(Debug, Clone, Copy, Default)]
pub struct PgDriver;

impl Driver for PgDriver {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn dialect(&self) -> DialectId {
        DialectId::Postgres
    }

    fn schemes(&self) -> &'static [&'static str] {
        &["postgres", "postgresql"]
    }

    fn connect<'a>(
        &'a self,
        config: &'a DbConfig,
    ) -> BoxFuture<'a, Result<Database, ConnectError>> {
        Box::pin(async move { PgPool::connect(config).await.map(Database::new) })
    }
}
