//! Connection pooling on `r2d2`, and the registry driver `postgres-sync`.
//!
//! URLs are anything `postgres::Config` parses, e.g.
//! `postgres://user:password@host:5432/db`. [`PgPool::connect`] uses rustls
//! when the `tls-rustls` feature is on (the URL's `sslmode` decides), and no
//! TLS otherwise; for another TLS setup, build a [`PgManager::with_tls`] and
//! a [`PgPool`] yourself.
//!
//! A connection is discarded when it is closed, or when it owes a rollback
//! that cannot be sent; `idle_timeout` and `max_lifetime` are enforced by
//! r2d2.

use std::fmt;
use std::sync::Arc;

use postgres::tls::{MakeTlsConnect, TlsConnect};
use postgres::{NoTls, Socket};
use rupa_core::Postgres;
use rupa_core::config::{DbConfig, PoolConfig};
use rupa_core::error::{ConnectError, DynError};
use rupa_core::pool::r2d2 as glue;
use rupa_core::pool::{OwnedTx, Pooled};
use rupa_core::tx::TxConnection;

use crate::{PgError, PgExecutor};

/// A connection checked out of a [`PgPool`].
pub type PgConn = Pooled<r2d2::PooledConnection<PgManager>>;

/// A transaction that owns its pooled connection (see [`Pool::begin`](rupa_core::pool::Pool::begin)).
pub type PgPoolTx = OwnedTx<PgConn>;

type ConnectFn =
    dyn Fn(&postgres::Config) -> Result<postgres::Client, postgres::Error> + Send + Sync;

/// Opens [`PgExecutor`]s for `r2d2`.
#[derive(Clone)]
pub struct PgManager {
    config: postgres::Config,
    connect: Arc<ConnectFn>,
    statement_cache_size: Option<usize>,
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
            statement_cache_size: None,
            connect: Arc::new(move |config: &postgres::Config| config.connect(tls.clone())),
        }
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
        Ok(self.wrap(PgExecutor::new((self.connect)(&self.config)?)))
    }

    fn is_valid(&self, conn: &mut PgExecutor) -> Result<(), PgError> {
        conn.clean()
    }

    fn has_broken(&self, conn: &mut PgExecutor) -> bool {
        conn.client().is_closed() || (conn.is_dirty() && conn.clean().is_err())
    }
}

rupa_core::r2d2_pool! {
    /// A pool of sync Postgres connections; a [`Pool`](rupa_core::pool::Pool), so `Repo<PgPool>` runs
    /// sync `&self` repositories and `pool.begin()?` opens an owned transaction.
    /// Cloning shares the pool.
    pub struct PgPool {
        manager: PgManager,
        dialect: Postgres,
        error: PgError,
    }
}

impl PgPool {
    /// Builds a pool from `manager` with `config`'s sizing and timeouts,
    /// opening the first connection (so bad credentials fail here).
    pub fn new(manager: PgManager, config: &PoolConfig) -> Result<Self, PgError> {
        Self::build(manager, glue::builder(config))
    }

    /// Builds a pool for a `postgres:` URL with the config's
    /// [`PostgresOptions`](rupa_core::config::PostgresOptions), without TLS.
    pub fn connect(config: &DbConfig) -> Result<Self, ConnectError> {
        let (pg, statement_cache_size) = rupa_driver_tokio_postgres::shared::pg_config(config)?;
        let pg = postgres::Config::from(pg);
        #[cfg(feature = "tls-rustls")]
        let manager = PgManager::with_tls(pg, rupa_driver_tokio_postgres::shared::rustls()?);
        #[cfg(not(feature = "tls-rustls"))]
        let manager = PgManager::new(pg);
        let manager = manager.statement_cache_size(statement_cache_size);

        Self::new(manager, &config.pool).map_err(|e| ConnectError::Connect(DynError::new(e)))
    }
}
