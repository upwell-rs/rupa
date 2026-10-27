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
//! key-value form. [`PgPool::connect`] uses rustls when the `tls-rustls`
//! feature is on (the URL's `sslmode` decides), and no TLS otherwise; for
//! another TLS setup, build a [`PgManager::with_tls`] and a [`PgPool`]
//! yourself.
//!
//! # Recycling
//!
//! A connection returned to the pool is discarded if it is closed, older
//! than `max_lifetime`, or idle for longer than `idle_timeout`. A connection
//! whose transaction was dropped while open is rolled back (cleaned) before
//! reuse, or discarded if that fails.

use std::fmt;
use std::sync::Arc;

use deadpool::managed::{self, Metrics, RecycleError, RecycleResult};
use rupa_core::Postgres;
use rupa_core::config::{DbConfig, PoolConfig};
use rupa_core::error::{ConnectError, DynError};
use rupa_core::exec::Awaited;
use rupa_core::exec::BoxFuture;
use rupa_core::pool::deadpool::{self as glue, Expiry};
use rupa_core::pool::{AsyncConnect, AsyncPool, Connect, OwnedAsyncTx, PoolStatus, Pooled};
use rupa_core::tx::AsyncTxConnection;
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
    statement_cache_size: Option<usize>,
    expiry: Expiry,
}

impl PgManager {
    /// Connects without TLS.
    pub fn new(config: tokio_postgres::Config) -> Self {
        Self::with_tls(config, NoTls)
    }

    /// Connects with `tls`, e.g. a `tokio-postgres-rustls` connector (see
    /// `shared::rustls` with the `tls-rustls` feature).
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
            statement_cache_size: None,
            connect: Arc::new(connect),
            expiry: Expiry::default(),
        }
    }

    /// Discards connections that are too old or were idle too long when
    /// they are recycled.
    pub fn expiry(mut self, expiry: Expiry) -> Self {
        self.expiry = expiry;
        self
    }

    /// Keeps at most `capacity` prepared statements per connection.
    pub fn statement_cache_size(mut self, capacity: Option<usize>) -> Self {
        self.statement_cache_size = capacity;
        self
    }

    fn wrap(&self, conn: PgExecutor) -> PgExecutor {
        match self.statement_cache_size {
            Some(size) => conn.with_statement_cache_size(size),
            None => conn,
        }
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
            .field("expiry", &self.expiry)
            .finish_non_exhaustive()
    }
}

impl managed::Manager for PgManager {
    type Type = PgExecutor;
    type Error = PgError;

    async fn create(&self) -> Result<PgExecutor, PgError> {
        let client = (self.connect)(self.config.clone()).await?;
        Ok(self.wrap(PgExecutor::new(client)))
    }

    async fn recycle(&self, conn: &mut PgExecutor, metrics: &Metrics) -> RecycleResult<PgError> {
        if conn.client().is_closed() {
            return Err(RecycleError::message("connection closed"));
        }

        if holds_migration_state(conn) {
            return Err(RecycleError::message(
                "migration run left state on the connection",
            ));
        }

        self.expiry.check(metrics)?;

        conn.clean().await.map_err(RecycleError::Backend)
    }
}

/// A pool of Postgres connections; an [`AsyncPool`], so repository views run
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
        let manager = manager.expiry(Expiry::from_config(config));
        let pool = glue::builder(manager, config, config.max_size)
            .build()
            .map_err(|e| PgError::Pool(Box::new(e)))?;
        let pool = Self::from_deadpool(pool);

        drop(pool.get().await?);

        Ok(pool)
    }

    /// Builds a pool for a `postgres:` URL with the config's
    /// [`PostgresOptions`](rupa_core::config::PostgresOptions). Uses rustls when
    /// `tls-rustls` is on (the URL's `sslmode` decides), and no TLS otherwise.
    pub async fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        Self::new(manager(config)?, &config.pool)
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
        glue::get(&self.pool, PgError::Pool).await
    }

    fn status(&self) -> PoolStatus {
        glue::status(&self.pool)
    }
}

/// Whether a migration run left state on `conn`, so the pool must discard it
/// instead of handing it out again.
fn holds_migration_state(conn: &PgExecutor) -> bool {
    #[cfg(feature = "migrations")]
    let active = conn.migration.is_active();
    #[cfg(not(feature = "migrations"))]
    let active = {
        let _ = conn;

        false
    };

    active
}

impl Connect for PgPool {
    type Completion = Awaited;
    type Connection = PgExecutor;
}

impl AsyncConnect for PgPool {
    async fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        Self::connect(config).await
    }

    async fn connect_one(config: &DbConfig) -> Result<PgExecutor, ConnectError> {
        managed::Manager::create(&manager(config)?)
            .await
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }
}

/// The connection manager `config` describes.
fn manager(config: &DbConfig) -> Result<PgManager, ConnectError> {
    let (pg, statement_cache_size) = crate::shared::pg_config(config)?;
    #[cfg(feature = "tls-rustls")]
    let manager = PgManager::with_tls(pg, crate::shared::rustls()?);
    #[cfg(not(feature = "tls-rustls"))]
    let manager = PgManager::new(pg);

    Ok(manager.statement_cache_size(statement_cache_size))
}
