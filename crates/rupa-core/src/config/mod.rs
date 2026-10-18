//! Database configuration, shared by every driver.
//!
//! A [`DbConfig`] says which database to connect to and how to pool
//! connections: shared fields (`url`, `pool`, `qualifier`) plus a
//! [`DriverConfig`], an enum with one variant per driver and that driver's
//! typed options. It is plain data, deserializable with serde (from TOML, or
//! a DI framework's configuration), and the
//! [`Registry`](crate::registry::Registry) turns it into a
//! [`Database`](crate::registry::Database) with whichever drivers are
//! compiled in. The variants exist whether or not their driver is compiled
//! in, so one configuration format serves every build.
//!
//! In serialized form the driver is a flattened tag, `driver`, and its
//! options sit next to the shared fields:
//!
//! ```toml
//! [database]
//! url = "sqlite://data/app.db"
//! driver = "sqlite"               # optional: inferred from the URL scheme
//! busy_timeout = "2s"             # SqliteOptions
//! journal_mode = "wal"
//!
//! [database.pool]
//! max_size = 16
//! acquire_timeout = "5s"          # a number of milliseconds, or "500ms", "5s", "2m", "1h"
//! idle_timeout = "10m"
//! max_lifetime = "off"            # "off" (or null) disables a timeout
//! ```
//!
//! | `driver` | variant | options | cargo feature of `rupa` |
//! |---|---|---|---|
//! | `postgres` | [`DriverConfig::Postgres`] | [`PostgresOptions`] | `postgres` (async) |
//! | `postgres-sync` | [`DriverConfig::PostgresSync`] | [`PostgresOptions`] | `postgres-sync` |
//! | `sqlite` | [`DriverConfig::Sqlite`] | [`SqliteOptions`] | `sqlite` |
//! | `mysql` | [`DriverConfig::MySql`] | [`MySqlOptions`] | `mysql` |
//! | `sqlx-postgres` | [`DriverConfig::SqlxPostgres`] | [`PostgresOptions`] | `sqlx-postgres` |
//! | `sqlx-sqlite` | [`DriverConfig::SqlxSqlite`] | [`SqliteOptions`] | `sqlx-sqlite` |
//! | `sqlx-mysql` | [`DriverConfig::SqlxMySql`] | [`MySqlOptions`] | `sqlx-mysql` |
//! | `memory` | [`DriverConfig::Memory`] | none | `memory` |
//! | anything else | [`DriverConfig::Other`] | any keys, as strings | a third-party driver |
//!
//! Without a `driver` tag:
//!
//! - with no driver options, the config is [`DriverConfig::Auto`], and the
//!   registry picks the first registered driver for the URL scheme (the
//!   async Postgres driver before the sync one);
//! - with driver options, the variant is inferred from the URL scheme
//!   (`postgres:` → `postgres`, `sqlite:` → `sqlite`, `mysql:` →
//!   `sqlx-mysql`), so the options are still checked against the right type.
//!   Each is the preferred driver for its scheme, as in the facade's
//!   registry order (see `docs/benchmarks.md`, "Drivers").
//!
//! A tag names a database's options and its preferred driver, not a hard
//! requirement: if that driver isn't compiled in, the registry uses another
//! one for the same database and URL scheme (so `driver = "sqlite"` works in
//! a build with only `sqlx-sqlite`). Every driver for a database passes the
//! same conformance suite and stores values the same way.
//!
//! Unknown keys are an error for every known driver.

mod driver;
mod duration;
mod options;
mod pool;
#[cfg(test)]
mod tests;

use std::fmt;
use std::str::FromStr;

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

pub use driver::DriverConfig;
pub use duration::{format_duration, parse_duration};
pub use options::{MySqlOptions, OpenMode, PostgresOptions, SqliteOptions, SqliteTarget};
pub use pool::PoolConfig;

/// Where and how to connect: a URL, pool settings, and the driver with its
/// options. See the [module docs](self) for the serialized form.
///
/// `Debug` and [`redacted_url`](Self::redacted_url) hide the password.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct DbConfig {
    /// The connection URL, e.g. `postgres://user:pw@host/db`,
    /// `sqlite://data/app.db`, `sqlite::memory:`, `mysql://..`, `memory:`.
    pub url: String,
    /// Pool sizing and timeouts.
    pub pool: PoolConfig,
    /// A name for this database when an application has several (`primary`,
    /// `replica`). rupa only carries it; DI integrations use it to qualify
    /// the database they provide.
    pub qualifier: Option<String>,
    /// The driver and its options.
    pub driver: DriverConfig,
}

impl DbConfig {
    /// A configuration for `url` with default pool settings; the driver is
    /// chosen by the URL scheme.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }

    /// Sets the driver and its options.
    pub fn driver(mut self, driver: impl Into<DriverConfig>) -> Self {
        self.driver = driver.into();
        self
    }

    /// Replaces the pool settings.
    pub fn pool(mut self, pool: PoolConfig) -> Self {
        self.pool = pool;
        self
    }

    /// Sets the pool's maximum size.
    pub fn max_connections(mut self, n: u32) -> Self {
        self.pool.max_size = n;
        self
    }

    /// Names this database (see [`qualifier`](Self::qualifier)).
    pub fn qualifier(mut self, name: impl Into<String>) -> Self {
        self.qualifier = Some(name.into());
        self
    }

    /// The URL scheme (`postgres` in `postgres://..`, `sqlite` in
    /// `sqlite::memory:`), lowercased.
    pub fn scheme(&self) -> Option<String> {
        scheme_of(&self.url)
    }

    /// The URL with its password replaced by `***`, for logs and errors.
    pub fn redacted_url(&self) -> String {
        redact(&self.url)
    }
}

impl FromStr for DbConfig {
    type Err = std::convert::Infallible;

    /// A configuration for the URL `s` with default pool settings.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl fmt::Debug for DbConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbConfig")
            .field("url", &self.redacted_url())
            .field("pool", &self.pool)
            .field("qualifier", &self.qualifier)
            .field("driver", &self.driver)
            .finish()
    }
}

fn scheme_of(url: &str) -> Option<String> {
    let (scheme, _) = url.split_once(':')?;
    let valid = scheme.chars().next()?.is_ascii_alphabetic()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Hides the password in `scheme://user:password@host/..`.
fn redact(url: &str) -> String {
    let Some(start) = url.find("://").map(|i| i + 3) else {
        return url.to_owned();
    };
    let authority_end = url[start..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |i| start + i);
    let Some(at) = url[start..authority_end].rfind('@').map(|i| start + i) else {
        return url.to_owned();
    };
    match url[start..at].find(':') {
        Some(colon) => format!("{}:***{}", &url[..start + colon], &url[at..]),
        None => url.to_owned(),
    }
}

impl Serialize for DbConfig {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let options = self.driver.options();
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("url", &self.url)?;
        map.serialize_entry("pool", &self.pool)?;
        if let Some(q) = &self.qualifier {
            map.serialize_entry("qualifier", q)?;
        }

        if let Some(name) = self.driver.name() {
            map.serialize_entry("driver", name)?;
        }

        for (key, value) in &options {
            map.serialize_entry(key, value)?;
        }

        map.end()
    }
}

impl<'de> Deserialize<'de> for DbConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Raw {
            url: String,
            pool: PoolConfig,
            qualifier: Option<String>,
            driver: Option<String>,
            #[serde(flatten)]
            options: serde_json::Map<String, serde_json::Value>,
        }

        let raw = Raw::deserialize(d)?;
        let name = match raw.driver {
            Some(name) => Some(name),
            None if raw.options.is_empty() => None,
            None => {
                let inferred = scheme_of(&raw.url)
                    .and_then(|s| DriverConfig::for_scheme(&s))
                    .and_then(|d| d.name().map(str::to_owned));
                Some(inferred.ok_or_else(|| {
                    let key = raw.options.keys().next().map_or("", String::as_str);
                    de::Error::custom(format!(
                        "unknown field `{key}`: driver options need a `driver`, \
                         or a URL scheme rupa knows"
                    ))
                })?)
            }
        };
        let driver = match name {
            None => DriverConfig::Auto,
            Some(name) => {
                DriverConfig::from_parts(&name, raw.options).map_err(de::Error::custom)?
            }
        };
        Ok(DbConfig {
            url: raw.url,
            pool: raw.pool,
            qualifier: raw.qualifier,
            driver,
        })
    }
}
