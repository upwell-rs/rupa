//! A database's configuration: separate connection fields, an optional
//! templated `url` override, the pool and the migration policy.

mod url;

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;

use rupa_core::config::{DbConfig, DriverConfig, PoolConfig};
use serde::Deserialize;
use upwell::schema::ConfigSchema;
use upwell::{ConfigProperties, DefaultSpec, config};

use super::Declaration;
use url::{Family, build, is_private_memory, with_userinfo};

/// One database's configuration, as written at its configuration path.
///
/// The connection is described field by field. `url` overrides those fields
/// with a complete URL; it goes through Upwell's templating like any value,
/// so it can reference them, as in
/// `url = "postgres://${database.primary.user}@${database.primary.host}/app"`,
/// or environment variables. Passwords never appear in `Debug` output.
///
/// It declares its schema, so Upwell's tooling lists every field.
#[config]
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseSettings {
    /// The rupa driver: `postgres`, `postgres-sync`, `mysql`, `sqlite`,
    /// `sqlx-postgres`, `sqlx-mysql`, `sqlx-sqlite`, or `memory`. Optional
    /// with `url`, whose scheme then picks the driver.
    pub driver: Option<String>,
    /// The server host of a networked database.
    pub host: Option<String>,
    /// The server port; the driver's default without one.
    pub port: Option<u16>,
    /// The user the application connects as.
    pub user: Option<String>,
    /// That user's password.
    pub password: Option<String>,
    /// The database name on the server.
    pub name: Option<String>,
    /// A SQLite database file, or `:memory:`.
    pub path: Option<String>,
    /// URL query parameters, such as `sslmode = "require"` or SQLite's
    /// `mode = "rwc"`.
    pub params: BTreeMap<String, String>,
    /// A complete connection URL that replaces the fields above.
    pub url: Option<String>,
    /// Pool sizing and timeouts.
    pub pool: PoolConfig,
    /// The driver's options, as in rupa's `DbConfig`.
    pub options: serde_json::Map<String, serde_json::Value>,
    /// How and with which credentials migrations run.
    pub migrations: MigrationSettings,
}

/// How a database's migrations run, and with which credentials.
///
/// The credentials replace the application's for migration sessions only,
/// so the running application can use a role without DDL rights. Without
/// them, migrations use the application's connection.
#[config]
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationSettings {
    /// What startup does before the database is handed out.
    pub startup: Startup,
    /// The user migrations connect as.
    pub user: Option<String>,
    /// That user's password.
    pub password: Option<String>,
    /// A complete migration connection URL, templated like `url`.
    pub url: Option<String>,
}

/// What application startup does with a database's migrations, before
/// anything that injects the database is built.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Startup {
    /// Nothing: the schema is managed elsewhere.
    Off,
    /// Fail startup unless every migration is installed and nothing blocks.
    #[default]
    Check,
    /// Apply pending migrations, then continue. Applies at most once per
    /// target and process: a database rebuilt by a configuration reload
    /// only checks.
    Apply,
}

/// Why a database's configuration cannot produce a connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// Neither a driver nor a URL names the database.
    #[error("set `driver` (or a complete `url`)")]
    NoDriver,
    /// The driver needs a field that is missing.
    #[error("the `{driver}` driver needs `{field}` (or a complete `url`)")]
    Missing {
        /// The configured driver.
        driver: String,
        /// The missing field.
        field: &'static str,
    },
    /// A field the driver does not use is set.
    #[error("the `{driver}` driver does not use `{field}`")]
    Unused {
        /// The configured driver.
        driver: String,
        /// The unused field.
        field: &'static str,
    },
    /// The driver is not one rupa can build a URL for.
    #[error("rupa cannot build a URL for the `{0}` driver; set a complete `url`")]
    Unknown(String),
    /// The driver options are invalid.
    #[error("invalid driver options: {0}")]
    Options(String),
    /// Migration credentials cannot be placed in the URL.
    #[error("cannot put migration credentials into `url`: {0}")]
    Credentials(String),
    /// Startup would check or apply migrations on a private in-memory SQLite
    /// database, which the migration connection and the pool each open
    /// afresh.
    #[error(
        "set `migrations.startup = \"off\"` for a private in-memory SQLite database: the migration connection and the pool would each open their own"
    )]
    PrivateMemory,
}

impl DatabaseSettings {
    /// The connection the running application uses.
    pub fn runtime(&self) -> Result<DbConfig, SettingsError> {
        let url = match &self.url {
            Some(url) => url.clone(),
            None => self.build(self.user.as_deref(), self.password.as_deref())?,
        };

        self.config(url)
    }

    /// The connection migrations use: `migrations.url`, or the runtime
    /// connection with `migrations.user` and `migrations.password` in place
    /// of the application's credentials.
    pub fn migration(&self) -> Result<DbConfig, SettingsError> {
        let migrations = &self.migrations;
        let url = match (&migrations.url, &migrations.user, &self.url) {
            (Some(url), _, _) => url.clone(),
            (None, Some(user), Some(url)) => {
                with_userinfo(url, user, migrations.password.as_deref())
                    .map_err(SettingsError::Credentials)?
            }
            (None, Some(user), None) => self.build(Some(user), migrations.password.as_deref())?,
            (None, None, _) => return self.runtime(),
        };

        self.config(url)
    }

    /// The startup policy, refused where it cannot work: checking or applying
    /// migrations on a private in-memory SQLite database would reach a
    /// database the pool never sees.
    pub fn startup(&self) -> Result<Startup, SettingsError> {
        let startup = self.migrations.startup;

        if startup != Startup::Off
            && (is_private_memory(&self.runtime()?.url)
                || is_private_memory(&self.migration()?.url))
        {
            return Err(SettingsError::PrivateMemory);
        }

        Ok(startup)
    }

    /// A `DbConfig` for `url` with this database's driver options and pool.
    pub(super) fn config(&self, url: String) -> Result<DbConfig, SettingsError> {
        let driver = match &self.driver {
            Some(driver) => DriverConfig::from_parts(driver, self.options.clone())
                .map_err(SettingsError::Options)?,
            None if self.options.is_empty() => DriverConfig::Auto,
            None => return Err(SettingsError::NoDriver),
        };

        Ok(DbConfig::new(url).driver(driver).pool(self.pool))
    }

    /// The URL the fields describe, with `user` and `password`.
    fn build(&self, user: Option<&str>, password: Option<&str>) -> Result<String, SettingsError> {
        let driver = self.driver.as_deref().ok_or(SettingsError::NoDriver)?;
        let family = Family::of(driver).ok_or_else(|| SettingsError::Unknown(driver.to_owned()))?;

        build(family, driver, self, user, password)
    }
}

impl fmt::Debug for DatabaseSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseSettings")
            .field("driver", &self.driver)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("name", &self.name)
            .field("path", &self.path)
            .field("params", &self.params)
            .field(
                "url",
                &self.url.as_deref().map(rupa_migrate::commands::redact_url),
            )
            .field("pool", &self.pool)
            .field("options", &self.options)
            .field("migrations", &self.migrations)
            .finish()
    }
}

impl fmt::Debug for MigrationSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationSettings")
            .field("startup", &self.startup)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field(
                "url",
                &self.url.as_deref().map(rupa_migrate::commands::redact_url),
            )
            .finish()
    }
}

/// The configuration of database `D`: its own type, so Upwell binds each
/// declared database separately.
#[derive(Deserialize)]
#[serde(transparent, bound = "")]
pub struct Settings<D> {
    settings: DatabaseSettings,
    #[serde(skip)]
    declaration: PhantomData<fn() -> D>,
}

impl<D> Deref for Settings<D> {
    type Target = DatabaseSettings;

    fn deref(&self) -> &DatabaseSettings {
        &self.settings
    }
}

impl<D> fmt::Debug for Settings<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.settings.fmt(f)
    }
}

/// Declares [`DatabaseSettings`]' schema and defaults, so tooling discovers
/// every field of each declared database.
impl<D: Declaration> ConfigProperties for Settings<D> {
    const NAME: &'static str = D::NAME;
    const DEFAULTS: DefaultSpec = <DatabaseSettings as ConfigProperties>::DEFAULTS;
    const SCHEMA: ConfigSchema = <DatabaseSettings as ConfigProperties>::SCHEMA;
}

#[cfg(test)]
mod tests;
