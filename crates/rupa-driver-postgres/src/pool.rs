//! Connection pooling on `r2d2`, and the registry driver `postgres-sync`.
//!
//! URLs are anything `postgres::Config` parses, e.g.
//! `postgres://user:password@host:5432/db`. The registry driver connects
//! without TLS; for TLS, build a [`PgManager::with_tls`] and a [`PgPool`]
//! yourself.
//!
//! A connection is discarded when it is closed, or when it owes a rollback
//! that cannot be sent; `idle_timeout` and `max_lifetime` are enforced by
//! r2d2.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use postgres::tls::{MakeTlsConnect, TlsConnect};
use postgres::{NoTls, Socket};
use rupa_core::config::{DbConfig, PoolConfig};
use rupa_core::erased::{BoxFuture, DynError};
use rupa_core::exec::Blocking;
use rupa_core::pool::{OwnedTx, Pool, PoolStatus, Pooled};
use rupa_core::registry::{ConnectError, Database, Driver};
use rupa_core::tx::TxConnection;
use rupa_core::{DialectId, Postgres};

use crate::{PgError, PgExecutor};

/// A connection checked out of a [`PgPool`].
pub type PgConn = Pooled<r2d2::PooledConnection<PgManager>>;

/// A transaction that owns its pooled connection (see [`Pool::begin`]).
pub type PgPoolTx = OwnedTx<PgConn>;

type ConnectFn =
    dyn Fn(&postgres::Config) -> Result<postgres::Client, postgres::Error> + Send + Sync;

/// Opens [`PgExecutor`]s for `r2d2`.
#[derive(Clone)]
pub struct PgManager {
    config: postgres::Config,
    connect: Arc<ConnectFn>,
}

impl PgManager {
    /// Connects without TLS.
    pub fn new(config: postgres::Config) -> Self {
        Self::with_tls(config, NoTls)
    }

    /// Connects with `tls`.
    pub fn with_tls<T>(config: postgres::Config, tls: T) -> Self
    where
        T: MakeTlsConnect<Socket> + Clone + Send + Sync + 'static,
        T::Stream: Send,
        T::TlsConnect: Send,
        <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
    {
        Self {
            config,
            connect: Arc::new(move |config: &postgres::Config| config.connect(tls.clone())),
        }
    }

    /// The connection settings.
    pub fn config(&self) -> &postgres::Config {
        &self.config
    }
}

impl fmt::Debug for PgManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgManager")
            .field("hosts", &self.config.get_hosts())
            .field("dbname", &self.config.get_dbname())
            .field("user", &self.config.get_user())
            .finish_non_exhaustive()
    }
}

impl r2d2::ManageConnection for PgManager {
    type Connection = PgExecutor;
    type Error = PgError;

    fn connect(&self) -> Result<PgExecutor, PgError> {
        Ok(PgExecutor::new((self.connect)(&self.config)?))
    }

    fn is_valid(&self, conn: &mut PgExecutor) -> Result<(), PgError> {
        conn.clean()
    }

    fn has_broken(&self, conn: &mut PgExecutor) -> bool {
        conn.client().is_closed() || (conn.is_dirty() && conn.clean().is_err())
    }
}

/// A pool of sync Postgres connections; a [`Pool`], so `Repo<PgPool>` runs
/// sync `&self` repositories and `pool.begin()?` opens an owned transaction.
/// Cloning shares the pool.
#[derive(Clone)]
pub struct PgPool {
    pool: r2d2::Pool<PgManager>,
    dialect: Postgres,
}

impl PgPool {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts,
    /// opening the first connection (so bad credentials fail here).
    pub fn new(manager: PgManager, config: &PoolConfig) -> Result<Self, PgError> {
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
            .map_err(|e| PgError::Pool(Box::new(e)))?;
        Ok(Self::from_r2d2(pool))
    }

    /// Builds a pool for a `postgres:` URL with the config's
    /// [`PostgresOptions`](rupa_core::config::PostgresOptions), without TLS.
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let mut pg = postgres::Config::from_str(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        let options = config.driver.postgres()?;
        if let Some(name) = &options.application_name {
            pg.application_name(name);
        }
        if let Some(timeout) = options.connect_timeout {
            pg.connect_timeout(timeout);
        }
        Self::new(PgManager::new(pg), &config.pool)
            .map_err(|e| ConnectError::Connect(DynError::new(e)))
    }

    /// Wraps an `r2d2` pool built elsewhere.
    pub fn from_r2d2(pool: r2d2::Pool<PgManager>) -> Self {
        Self {
            pool,
            dialect: Postgres,
        }
    }

    /// The underlying `r2d2` pool.
    pub fn r2d2(&self) -> &r2d2::Pool<PgManager> {
        &self.pool
    }
}

impl fmt::Debug for PgPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgPool")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

/// r2d2 has no "wait forever"; a century will do.
fn timeout(t: Option<Duration>) -> Duration {
    t.filter(|t| !t.is_zero())
        .unwrap_or(Duration::from_secs(100 * 365 * 24 * 3600))
}

impl Pool for PgPool {
    type Conn = PgConn;

    fn dialect(&self) -> &Postgres {
        &self.dialect
    }

    fn get(&self) -> Result<PgConn, PgError> {
        self.pool
            .get()
            .map(Pooled::new)
            .map_err(|e| PgError::Pool(Box::new(e)))
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

/// The registry entry for this driver: name `postgres-sync`, schemes `postgres`
/// and `postgresql`. Builds a [`PgPool`] (without TLS), served to async code
/// through [`Blocking`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PgDriver;

impl Driver for PgDriver {
    fn name(&self) -> &'static str {
        "postgres-sync"
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
        let database = PgPool::connect(config).map(|pool| Database::new(Blocking(pool)));
        Box::pin(std::future::ready(database))
    }
}
