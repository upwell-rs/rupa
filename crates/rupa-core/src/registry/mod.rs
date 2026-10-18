//! Choosing a driver from configuration, DI-neutrally.
//!
//! - A [`Driver`] describes one driver crate: its name (the `driver` tag of
//!   [`DriverConfig`](crate::config::DriverConfig)), dialect, URL schemes, and how to build a pooled
//!   [`Database`] from a [`DbConfig`]. Each driver crate exports one (e.g.
//!   `rupa_driver_rusqlite::SqliteDriver`) behind its `dynamic` feature.
//! - A [`Registry`] holds the drivers an application compiled in and
//!   resolves a [`DbConfig`] to one of them: by name when the config names a
//!   driver, otherwise by URL scheme (the first registered driver for the
//!   scheme wins). Registration is explicit: no link-time magic. The `rupa`
//!   facade builds one, once, from its cargo features (`rupa::registry()`,
//!   `rupa::connect(..)`).
//! - A [`Database`] is the result: a cheaply cloneable, type-erased async
//!   connection pool (`Arc` inside). It is an [`AcquireAsync`](crate::repo::AcquireAsync) source, so
//!   `Repo<Database>` runs `&self` repositories, and it opens owned,
//!   `'static` transactions for request-scoped use.
//!
//! Building is always async and fallible (one registry, not mirrored sync
//! and async ones): sync drivers are served through [`Blocking`](crate::exec::Blocking), as for
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

mod database;
mod options;

use std::fmt;
use std::sync::Arc;

use crate::config::DbConfig;
use crate::dialect::DialectId;
use crate::error::ConnectError;
use crate::exec::BoxFuture;

pub use database::Database;
pub use options::DriverOptions;

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

    /// The driver `config` selects:
    ///
    /// - by name if it names one (see
    ///   [`DriverConfig::name`](crate::config::DriverConfig::name));
    /// - if that driver isn't registered but the name is one rupa knows, the
    ///   first registered driver for the same database and URL scheme (see
    ///   [`DriverConfig::dialect`](crate::config::DriverConfig::dialect));
    /// - otherwise the first registered driver for its URL scheme.
    pub fn resolve(&self, config: &DbConfig) -> Result<&dyn Driver, ConnectError> {
        let scheme = config.scheme();
        if let Some(name) = config.driver.name() {
            let accepts_scheme =
                |d: &dyn Driver| scheme.as_deref().is_none_or(|s| d.schemes().contains(&s));
            let driver = self
                .drivers()
                .find(|d| d.name() == name)
                .or_else(|| {
                    let dialect = config.driver.dialect()?;
                    self.drivers()
                        .find(|d| d.dialect() == dialect && accepts_scheme(*d))
                })
                .ok_or_else(|| ConnectError::UnknownDriver {
                    name: name.to_owned(),
                    available: self.available(),
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

/// Declares a sync driver's registry entry: a unit struct whose `connect`
/// builds the driver's pool with `<pool>::connect(&DbConfig)` and serves it
/// to async code through [`Blocking`](crate::exec::Blocking) (statements run
/// on the calling thread).
#[macro_export]
#[doc(hidden)]
macro_rules! blocking_driver {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            name: $driver:literal,
            dialect: $dialect:ident,
            schemes: [$($scheme:literal),+ $(,)?],
            pool: $pool:ty $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default)]
        $vis struct $name;

        impl $crate::registry::Driver for $name {
            fn name(&self) -> &'static str {
                $driver
            }

            fn dialect(&self) -> $crate::DialectId {
                $crate::DialectId::$dialect
            }

            fn schemes(&self) -> &'static [&'static str] {
                &[$($scheme),+]
            }

            fn connect<'a>(
                &'a self,
                config: &'a $crate::config::DbConfig,
            ) -> $crate::exec::BoxFuture<
                'a,
                ::std::result::Result<$crate::registry::Database, $crate::error::ConnectError>,
            > {
                let database = <$pool>::connect(config)
                    .map(|pool| $crate::registry::Database::new($crate::exec::Blocking(pool)));

                ::std::boxed::Box::pin(::std::future::ready(database))
            }
        }
    };
}
