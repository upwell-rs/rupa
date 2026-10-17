//! Choosing a driver from configuration, DI-neutrally.
//!
//! - A [`Driver`] describes one driver crate: its name (the `driver` tag of
//!   [`DriverConfig`]), dialect, URL schemes, and how to build a pooled
//!   [`Database`] from a [`DbConfig`]. Each driver crate exports one (e.g.
//!   `rupa_driver_rusqlite::SqliteDriver`) behind its `pool` feature.
//! - A [`Registry`] holds the drivers an application compiled in and
//!   resolves a [`DbConfig`] to one of them: by name when the config names a
//!   driver, otherwise by URL scheme (the first registered driver for the
//!   scheme wins). Registration is explicit: no link-time magic. The `rupa`
//!   facade builds one, once, from its cargo features (`rupa::registry()`,
//!   `rupa::connect(..)`).
//! - A [`Database`] is the result: a cheaply cloneable, type-erased async
//!   connection pool (`Arc` inside). It is an [`AcquireAsync`] source, so
//!   `Repo<Database>` runs `&self` repositories, and it opens owned,
//!   `'static` transactions for request-scoped use.
//!
//! Building is always async and fallible (one registry, not mirrored sync
//! and async ones): sync drivers are served through [`Blocking`], as for
//! repositories.
//!
//! # Several databases
//!
//! Give each [`DbConfig`] a [`qualifier`](DbConfig::qualifier) (`primary`,
//! `replica`) and connect each one; the registry is stateless, so the same
//! one serves all of them:
//!
//! ```ignore
//! let registry = rupa::registry();
//! let primary = registry.connect(&config.primary).await?;
//! let replica = registry.connect(&config.replica).await?;
//! let reads: Arc<dyn UserRepository> = Arc::new(Repo::new(replica));
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{DbConfig, DriverConfig, parse_duration};
use crate::dialect::{Dialect, DialectId, DynDialect};
use crate::erased::{BoxAsyncExecutor, BoxAsyncTx, BoxFuture, DynError};
use crate::exec::Blocking;
use crate::pool::{AsyncPool, PoolStatus, SingleConnection};
use crate::repo::AcquireAsync;
use crate::security::{SecureTx, SecurityContext, apply_security, check_native_rls};
use crate::tx::{AsyncTx, AsyncTxConnection, TxOptions};

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// A pool behind a dyn-compatible interface; implemented for every
/// [`AsyncPool`].
trait DynPool: Send + Sync {
    fn status(&self) -> PoolStatus;
    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>>;
    fn begin_dyn(&self, options: TxOptions)
    -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>>;
}

impl<P: AsyncPool> DynPool for P {
    fn status(&self) -> PoolStatus {
        AsyncPool::status(self)
    }

    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>> {
        Box::pin(async move {
            let conn = self.get().await.map_err(DynError::new)?;
            Ok(BoxAsyncExecutor::new(conn))
        })
    }

    fn begin_dyn(
        &self,
        options: TxOptions,
    ) -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>> {
        Box::pin(async move {
            let conn = self.get().await.map_err(DynError::new)?;
            let tx = AsyncTx::begin(Box::new(conn), options)
                .await
                .map_err(DynError::new)?;
            Ok(BoxAsyncTx::new(tx))
        })
    }
}

/// A database chosen at run time: a type-erased async connection pool.
///
/// Cloning is cheap and shares the pool. Typical uses:
///
/// ```ignore
/// let db = rupa::connect(&DbConfig::new("sqlite://app.db")).await?;
///
/// // `&self` repositories: one pooled connection per call.
/// let users: Arc<dyn UserRepository> = Arc::new(Repo::new(db.clone()));
///
/// // A unit of work: an owned ('static) transaction on one connection.
/// let mut tx = db.begin().await?;
/// Repo::new(&mut tx).rename(id, "x").await?;
/// tx.commit().await?;
/// ```
#[derive(Clone)]
pub struct Database {
    pool: Arc<dyn DynPool>,
    dialect: DynDialect,
}

impl Database {
    /// Erases an async pool.
    pub fn new(pool: impl AsyncPool) -> Self {
        let dialect = DynDialect::new(pool.dialect().id());
        Self {
            pool: Arc::new(pool),
            dialect,
        }
    }

    /// One async connection, shared: callers take turns (see
    /// [`SingleConnection`]).
    pub fn single<E>(executor: E) -> Self
    where
        E: AsyncTxConnection + 'static,
        E::Dialect: Clone,
    {
        Self::new(SingleConnection::new(executor))
    }

    /// One sync connection, shared, driven through [`Blocking`].
    pub fn single_blocking<E>(executor: E) -> Self
    where
        E: crate::tx::TxConnection + 'static,
        E::Dialect: Clone,
    {
        Self::single(Blocking(executor))
    }

    /// The database's dialect, for runtime capability checks.
    pub fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    /// The pool's current occupancy.
    pub fn status(&self) -> PoolStatus {
        self.pool.status()
    }

    /// Checks a connection out of the pool. It returns when dropped.
    pub async fn get(&self) -> Result<BoxAsyncExecutor, DynError> {
        self.pool.get_dyn().await
    }

    /// Opens a transaction that owns its pooled connection, so it is
    /// `'static` (e.g. for a request scope). Dropped while open, it is rolled
    /// back before the connection is reused.
    pub async fn begin_with(&self, options: TxOptions) -> Result<BoxAsyncTx<'static>, DynError> {
        self.pool.begin_dyn(options).await
    }

    /// [`begin_with`](Self::begin_with) default options.
    pub async fn begin(&self) -> Result<BoxAsyncTx<'static>, DynError> {
        self.begin_with(TxOptions::default()).await
    }

    /// Opens an owned transaction with `context` applied (see
    /// [`crate::security`]). Checked at run time: fails with
    /// [`TxError::Unsupported`](crate::TxError::Unsupported), before
    /// touching the pool, unless the dialect has native row-level security.
    pub async fn try_begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<BoxAsyncTx<'static>>, DynError> {
        check_native_rls(&self.dialect)?;
        apply_security(self.begin_with(options).await?, context).await
    }
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("dialect", &self.dialect.id())
            .field("status", &self.status())
            .finish()
    }
}

impl AcquireAsync for Database {
    type Dialect = DynDialect;
    type Error = DynError;
    type Conn<'a> = BoxAsyncExecutor;

    fn acquire(
        &self,
    ) -> impl std::future::Future<Output = Result<BoxAsyncExecutor, DynError>> + Send {
        self.pool.get_dyn()
    }
}

// ---------------------------------------------------------------------------
// Drivers and the registry
// ---------------------------------------------------------------------------

/// One driver crate, as the [`Registry`] sees it.
pub trait Driver: Send + Sync + 'static {
    /// The name `DbConfig::driver` selects it by, e.g. `"rusqlite"`.
    fn name(&self) -> &'static str;

    /// The dialect of the databases it connects to.
    fn dialect(&self) -> DialectId;

    /// URL schemes it accepts, lowercase, e.g. `["sqlite"]`.
    fn schemes(&self) -> &'static [&'static str];

    /// Builds a pool from `config`. Connects eagerly (at least one
    /// connection), so a wrong URL or password fails here, not on first use.
    fn connect<'a>(&'a self, config: &'a DbConfig)
    -> BoxFuture<'a, Result<Database, ConnectError>>;
}

impl fmt::Debug for dyn Driver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Driver")
            .field("name", &self.name())
            .field("dialect", &self.dialect())
            .field("schemes", &self.schemes())
            .finish()
    }
}

/// The drivers available to an application; see the [module docs](self).
#[derive(Clone, Default)]
pub struct Registry {
    drivers: Vec<Arc<dyn Driver>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `driver`, after those already registered.
    pub fn with(mut self, driver: impl Driver) -> Self {
        self.register(driver);
        self
    }

    /// Adds `driver`, after those already registered.
    pub fn register(&mut self, driver: impl Driver) -> &mut Self {
        self.drivers.push(Arc::new(driver));
        self
    }

    /// The registered drivers, in registration order.
    pub fn drivers(&self) -> impl Iterator<Item = &dyn Driver> {
        self.drivers.iter().map(|d| &**d)
    }

    /// The driver `config` selects: by name if it names one (see
    /// [`DriverConfig::name`](crate::config::DriverConfig::name)), otherwise
    /// the first registered driver for its URL scheme.
    pub fn resolve(&self, config: &DbConfig) -> Result<&dyn Driver, ConnectError> {
        let scheme = config.scheme();
        if let Some(name) = config.driver.name() {
            let driver = self.drivers().find(|d| d.name() == name).ok_or_else(|| {
                ConnectError::UnknownDriver {
                    name: name.to_owned(),
                    available: self.available(),
                }
            })?;
            if let Some(scheme) = &scheme
                && !driver.schemes().contains(&scheme.as_str())
            {
                return Err(ConnectError::InvalidUrl(format!(
                    "driver `{name}` does not accept `{scheme}:` URLs (it accepts: {})",
                    driver.schemes().join(", ")
                )));
            }
            return Ok(driver);
        }
        let scheme = scheme.ok_or_else(|| {
            ConnectError::InvalidUrl(format!(
                "`{}` has no scheme (expected e.g. `postgres://..` or `sqlite:..`)",
                config.redacted_url()
            ))
        })?;
        self.drivers()
            .find(|d| d.schemes().contains(&scheme.as_str()))
            .ok_or_else(|| ConnectError::UnknownScheme {
                scheme,
                available: self.available(),
            })
    }

    /// Resolves `config` and connects with the chosen driver.
    pub async fn connect(&self, config: &DbConfig) -> Result<Database, ConnectError> {
        if config.pool.max_size == 0 {
            return Err(ConnectError::InvalidOption {
                key: "pool.max_size".into(),
                message: "must be at least 1".into(),
            });
        }
        self.resolve(config)?.connect(config).await
    }

    fn available(&self) -> Vec<String> {
        self.drivers()
            .map(|d| format!("{} ({})", d.name(), d.schemes().join(", ")))
            .collect()
    }
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.drivers()).finish()
    }
}

/// Why a [`DbConfig`] could not be turned into a [`Database`].
#[derive(Debug)]
#[non_exhaustive]
pub enum ConnectError {
    /// `driver` names no registered driver.
    UnknownDriver {
        /// The requested name.
        name: String,
        /// Registered drivers, as `name (schemes)`.
        available: Vec<String>,
    },
    /// No registered driver accepts the URL's scheme.
    UnknownScheme {
        /// The URL's scheme.
        scheme: String,
        /// Registered drivers, as `name (schemes)`.
        available: Vec<String>,
    },
    /// The URL is malformed or doesn't fit the driver.
    InvalidUrl(String),
    /// A pool setting or driver option is invalid or unknown.
    InvalidOption {
        /// The option's key.
        key: String,
        /// What is wrong with it.
        message: String,
    },
    /// The driver could not open its first connection.
    Connect(DynError),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |v: &[String]| {
            if v.is_empty() {
                "none; enable a driver feature of `rupa` (sqlite, postgres, mysql, ..)".to_owned()
            } else {
                v.join(", ")
            }
        };
        match self {
            ConnectError::UnknownDriver { name, available } => {
                write!(
                    f,
                    "no driver named `{name}` is registered (registered: {})",
                    list(available)
                )
            }
            ConnectError::UnknownScheme { scheme, available } => write!(
                f,
                "no registered driver accepts `{scheme}:` URLs (registered: {})",
                list(available)
            ),
            ConnectError::InvalidUrl(m) => write!(f, "invalid database URL: {m}"),
            ConnectError::InvalidOption { key, message } => {
                write!(f, "invalid database option `{key}`: {message}")
            }
            ConnectError::Connect(e) => write!(f, "could not connect: {e}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectError::Connect(e) => Some(e),
            _ => None,
        }
    }
}

/// Reads the untyped options of a third-party driver
/// ([`DriverConfig::Other`]), so that
/// keys the driver doesn't know are reported instead of silently ignored.
///
/// ```ignore
/// let mut opts = DriverOptions::new(&config.driver);
/// let busy = opts.duration("busy_timeout")?.unwrap_or(Duration::from_secs(5));
/// opts.finish()?; // errors on any key not read above
/// ```
#[derive(Debug)]
pub struct DriverOptions<'a> {
    remaining: BTreeMap<&'a str, &'a str>,
}

impl<'a> DriverOptions<'a> {
    /// Starts reading `driver`'s options; there are none unless it is
    /// [`DriverConfig::Other`].
    pub fn new(driver: &'a DriverConfig) -> Self {
        let remaining = match driver {
            DriverConfig::Other { options, .. } => options
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            _ => BTreeMap::new(),
        };
        Self { remaining }
    }

    /// Takes a string option.
    pub fn string(&mut self, key: &str) -> Option<&'a str> {
        self.remaining.remove(key)
    }

    /// Takes a duration option (see [`parse_duration`]).
    pub fn duration(&mut self, key: &str) -> Result<Option<Duration>, ConnectError> {
        self.parse(key, parse_duration)
    }

    /// Takes a boolean option (`true`/`false`/`on`/`off`/`1`/`0`).
    pub fn bool(&mut self, key: &str) -> Result<Option<bool>, ConnectError> {
        self.parse(key, |v| match v.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "1" | "yes" => Ok(true),
            "false" | "off" | "0" | "no" => Ok(false),
            other => Err(format!("expected a boolean, got `{other}`")),
        })
    }

    /// Takes an option and parses it with `parse`.
    pub fn parse<T>(
        &mut self,
        key: &str,
        parse: impl FnOnce(&str) -> Result<T, String>,
    ) -> Result<Option<T>, ConnectError> {
        self.string(key)
            .map(|v| {
                parse(v).map_err(|message| ConnectError::InvalidOption {
                    key: key.to_owned(),
                    message,
                })
            })
            .transpose()
    }

    /// Fails on the first option nobody took.
    pub fn finish(self) -> Result<(), ConnectError> {
        match self.remaining.into_keys().next() {
            None => Ok(()),
            Some(key) => Err(ConnectError::InvalidOption {
                key: key.to_owned(),
                message: "unknown option for this driver".into(),
            }),
        }
    }
}
