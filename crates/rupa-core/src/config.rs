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
//!   (`postgres:` → `postgres`, `sqlite:` → `sqlite`, `mysql:` → `mysql`), so
//!   the options are still checked against the right type.
//!
//! A tag names a database's options and its preferred driver, not a hard
//! requirement: if that driver isn't compiled in, the registry uses another
//! one for the same database and URL scheme (so `driver = "sqlite"` works in
//! a build with only `sqlx-sqlite`). Every driver for a database passes the
//! same conformance suite and stores values the same way.
//!
//! Unknown keys are an error for every known driver.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};

use crate::DialectId;
use crate::registry::ConnectError;

// ---------------------------------------------------------------------------
// DbConfig
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Drivers
// ---------------------------------------------------------------------------

/// Which driver to use, with its options. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DriverConfig {
    /// Chosen by the URL scheme, with default options.
    #[default]
    Auto,
    /// Async Postgres (`tokio-postgres`).
    Postgres(PostgresOptions),
    /// Sync Postgres (`postgres`).
    PostgresSync(PostgresOptions),
    /// SQLite (`rusqlite`).
    Sqlite(SqliteOptions),
    /// MySQL (`mysql`).
    MySql(MySqlOptions),
    /// Async Postgres on sqlx.
    SqlxPostgres(PostgresOptions),
    /// Async SQLite on sqlx.
    SqlxSqlite(SqliteOptions),
    /// Async MySQL on sqlx.
    SqlxMySql(MySqlOptions),
    /// The in-memory backend.
    Memory,
    /// A driver rupa doesn't know, with untyped options (read them with
    /// [`DriverOptions`](crate::registry::DriverOptions)).
    Other {
        /// The registered driver's name.
        name: String,
        /// Its options; non-string values are kept in their JSON form.
        options: BTreeMap<String, String>,
    },
}

impl DriverConfig {
    /// The `driver` tag: the registered driver's name, or `None` for
    /// [`Auto`](Self::Auto).
    pub fn name(&self) -> Option<&str> {
        Some(match self {
            DriverConfig::Auto => return None,
            DriverConfig::Postgres(_) => "postgres",
            DriverConfig::PostgresSync(_) => "postgres-sync",
            DriverConfig::Sqlite(_) => "sqlite",
            DriverConfig::MySql(_) => "mysql",
            DriverConfig::SqlxPostgres(_) => "sqlx-postgres",
            DriverConfig::SqlxSqlite(_) => "sqlx-sqlite",
            DriverConfig::SqlxMySql(_) => "sqlx-mysql",
            DriverConfig::Memory => "memory",
            DriverConfig::Other { name, .. } => name,
        })
    }

    /// The database a known driver connects to; `None` for
    /// [`Auto`](Self::Auto) and [`Other`](Self::Other).
    pub fn dialect(&self) -> Option<DialectId> {
        Some(match self {
            DriverConfig::Auto | DriverConfig::Other { .. } => return None,
            DriverConfig::Postgres(_)
            | DriverConfig::PostgresSync(_)
            | DriverConfig::SqlxPostgres(_) => DialectId::Postgres,
            DriverConfig::Sqlite(_) | DriverConfig::SqlxSqlite(_) => DialectId::Sqlite,
            DriverConfig::MySql(_) | DriverConfig::SqlxMySql(_) => DialectId::MySql,
            DriverConfig::Memory => DialectId::Memory,
        })
    }

    /// The default variant for a URL scheme, if rupa knows one.
    pub fn for_scheme(scheme: &str) -> Option<Self> {
        Some(match scheme {
            "postgres" | "postgresql" => DriverConfig::Postgres(PostgresOptions::default()),
            "sqlite" => DriverConfig::Sqlite(SqliteOptions::default()),
            "mysql" => DriverConfig::MySql(MySqlOptions::default()),
            "memory" => DriverConfig::Memory,
            _ => return None,
        })
    }

    /// Builds the variant tagged `name` from its serialized options.
    pub fn from_parts(
        name: &str,
        options: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, String> {
        fn typed<T: serde::de::DeserializeOwned>(
            name: &str,
            options: serde_json::Map<String, serde_json::Value>,
        ) -> Result<T, String> {
            serde_json::from_value(serde_json::Value::Object(options))
                .map_err(|e| format!("invalid options for driver `{name}`: {e}"))
        }
        Ok(match name {
            "postgres" => DriverConfig::Postgres(typed(name, options)?),
            "postgres-sync" => DriverConfig::PostgresSync(typed(name, options)?),
            "sqlite" => DriverConfig::Sqlite(typed(name, options)?),
            "mysql" => DriverConfig::MySql(typed(name, options)?),
            "sqlx-postgres" => DriverConfig::SqlxPostgres(typed(name, options)?),
            "sqlx-sqlite" => DriverConfig::SqlxSqlite(typed(name, options)?),
            "sqlx-mysql" => DriverConfig::SqlxMySql(typed(name, options)?),
            "memory" => match options.keys().next() {
                None => DriverConfig::Memory,
                Some(key) => return Err(format!("the memory driver has no option `{key}`")),
            },
            other => DriverConfig::Other {
                name: other.to_owned(),
                options: options
                    .into_iter()
                    .map(|(k, v)| match v {
                        serde_json::Value::String(s) => (k, s),
                        v => (k, v.to_string()),
                    })
                    .collect(),
            },
        })
    }

    /// The driver's options in serialized form (without the tag).
    pub fn options(&self) -> serde_json::Map<String, serde_json::Value> {
        let value = match self {
            DriverConfig::Auto | DriverConfig::Memory => return serde_json::Map::new(),
            DriverConfig::Postgres(o)
            | DriverConfig::PostgresSync(o)
            | DriverConfig::SqlxPostgres(o) => serde_json::to_value(o),
            DriverConfig::Sqlite(o) | DriverConfig::SqlxSqlite(o) => serde_json::to_value(o),
            DriverConfig::MySql(o) | DriverConfig::SqlxMySql(o) => serde_json::to_value(o),
            DriverConfig::Other { options, .. } => serde_json::to_value(options),
        };
        match value {
            Ok(serde_json::Value::Object(map)) => map,
            _ => unreachable!("driver options serialize as maps"),
        }
    }

    /// Postgres options, for the Postgres drivers: the configured ones (of
    /// any Postgres variant), or the defaults for [`Auto`](Self::Auto).
    pub fn postgres(&self) -> Result<PostgresOptions, ConnectError> {
        match self {
            DriverConfig::Auto => Ok(PostgresOptions::default()),
            DriverConfig::Postgres(o)
            | DriverConfig::PostgresSync(o)
            | DriverConfig::SqlxPostgres(o) => Ok(o.clone()),
            other => Err(other.mismatch("postgres")),
        }
    }

    /// SQLite options; see [`postgres`](Self::postgres).
    pub fn sqlite(&self) -> Result<SqliteOptions, ConnectError> {
        match self {
            DriverConfig::Auto => Ok(SqliteOptions::default()),
            DriverConfig::Sqlite(o) | DriverConfig::SqlxSqlite(o) => Ok(o.clone()),
            other => Err(other.mismatch("sqlite")),
        }
    }

    /// MySQL options; see [`postgres`](Self::postgres).
    pub fn mysql(&self) -> Result<MySqlOptions, ConnectError> {
        match self {
            DriverConfig::Auto => Ok(MySqlOptions::default()),
            DriverConfig::MySql(o) | DriverConfig::SqlxMySql(o) => Ok(o.clone()),
            other => Err(other.mismatch("mysql")),
        }
    }

    fn mismatch(&self, expected: &str) -> ConnectError {
        ConnectError::InvalidOption {
            key: "driver".into(),
            message: format!(
                "a `{expected}` driver was given options for `{}`",
                self.name().unwrap_or("auto")
            ),
        }
    }
}

impl From<PostgresOptions> for DriverConfig {
    /// Async Postgres with these options.
    fn from(o: PostgresOptions) -> Self {
        DriverConfig::Postgres(o)
    }
}

impl From<SqliteOptions> for DriverConfig {
    fn from(o: SqliteOptions) -> Self {
        DriverConfig::Sqlite(o)
    }
}

impl From<MySqlOptions> for DriverConfig {
    fn from(o: MySqlOptions) -> Self {
        DriverConfig::MySql(o)
    }
}

/// Options of both Postgres drivers. Most connection settings go in the URL
/// (`?sslmode=..&options=..`); these are the ones worth naming.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PostgresOptions {
    /// Reported in `pg_stat_activity`. Overrides the URL's.
    pub application_name: Option<String>,
    /// How long opening one connection may take. Overrides the URL's.
    #[serde(with = "duration")]
    pub connect_timeout: Option<Duration>,
    /// Server-side prepared statements kept per connection (an LRU). `0`
    /// prepares every query anew. Default: the driver's (256).
    pub statement_cache_size: Option<usize>,
}

/// SQLite options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SqliteOptions {
    /// How long a statement waits for another connection's lock. Default: 5 s.
    #[serde(with = "duration")]
    pub busy_timeout: Option<Duration>,
    /// `PRAGMA foreign_keys`, so references are enforced as in Postgres.
    /// Default: on.
    pub foreign_keys: bool,
    /// `PRAGMA journal_mode`, e.g. `wal` for concurrent readers. Default:
    /// SQLite's.
    pub journal_mode: Option<String>,
    /// Prepared statements kept per connection (an LRU). `0` prepares every
    /// query anew. Default: the driver's (128).
    pub statement_cache_size: Option<usize>,
}

impl Default for SqliteOptions {
    fn default() -> Self {
        Self {
            busy_timeout: Some(Duration::from_secs(5)),
            foreign_keys: true,
            journal_mode: None,
            statement_cache_size: None,
        }
    }
}

/// The database a `sqlite:` URL names, as every SQLite driver reads it:
///
/// | URL | database |
/// |---|---|
/// | `sqlite::memory:`, `sqlite://:memory:` | a private in-memory database (pools keep exactly one connection) |
/// | `sqlite://data/app.db`, `sqlite:data/app.db` | a file, relative to the working directory |
/// | `sqlite:///var/lib/app.db`, `sqlite://C:/data/app.db` | an absolute path |
///
/// A `?mode=` query parameter sets how a file is opened: `rwc` (read-write,
/// create if missing; the default), `rw`, or `ro`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqliteTarget {
    /// A private in-memory database.
    Memory,
    /// A database file.
    File {
        /// Its path.
        path: std::path::PathBuf,
        /// How to open it.
        mode: OpenMode,
    },
}

/// How a database file is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenMode {
    /// Read-only.
    ReadOnly,
    /// Read-write; the file must exist.
    ReadWrite,
    /// Read-write, created if missing.
    #[default]
    ReadWriteCreate,
}

impl SqliteTarget {
    /// Parses a `sqlite:` URL; see the [type docs](Self).
    pub fn parse(url: &str) -> Result<Self, String> {
        let rest = url
            .get(..7)
            .filter(|s| s.eq_ignore_ascii_case("sqlite:"))
            .map(|_| &url[7..])
            .ok_or_else(|| format!("`{url}` is not a sqlite: URL"))?;
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        if path == ":memory:" {
            return Ok(SqliteTarget::Memory);
        }
        if path.is_empty() {
            return Err("missing database path (use `sqlite::memory:` for memory)".into());
        }
        let mut mode = OpenMode::default();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            mode = match pair {
                "mode=ro" => OpenMode::ReadOnly,
                "mode=rw" => OpenMode::ReadWrite,
                "mode=rwc" => OpenMode::ReadWriteCreate,
                other => return Err(format!("unsupported URL parameter `{other}`")),
            };
        }
        Ok(SqliteTarget::File {
            path: std::path::PathBuf::from(path),
            mode,
        })
    }
}

/// MySQL options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MySqlOptions {
    /// Server-side prepared statements kept per connection (an LRU, kept by
    /// the `mysql` crate). Default: the crate's (32).
    pub statement_cache_size: Option<usize>,
    /// Statements run on every new connection, e.g.
    /// `"SET time_zone = '+00:00'"`.
    pub init: Vec<String>,
}

// ---------------------------------------------------------------------------
// Serde: shared fields plus a flattened, tagged driver
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Pool
// ---------------------------------------------------------------------------

/// Pool sizing and timeouts. Every driver maps these onto its pool; a driver
/// without pooling (the in-memory one) ignores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PoolConfig {
    /// The most connections open at once. Default: 10.
    pub max_size: u32,
    /// Idle connections to keep open, if the pool supports it. Default: none.
    pub min_idle: Option<u32>,
    /// How long `get` waits for a connection (including connecting) before
    /// failing. `None` waits forever. Default: 30 s.
    #[serde(with = "duration")]
    pub acquire_timeout: Option<Duration>,
    /// Idle connections older than this are closed. Default: 10 min.
    #[serde(with = "duration")]
    pub idle_timeout: Option<Duration>,
    /// Connections older than this are closed when returned. Default: 30 min.
    #[serde(with = "duration")]
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_size: 10,
            min_idle: None,
            acquire_timeout: Some(Duration::from_secs(30)),
            idle_timeout: Some(Duration::from_secs(10 * 60)),
            max_lifetime: Some(Duration::from_secs(30 * 60)),
        }
    }
}

impl PoolConfig {
    /// Sets [`max_size`](Self::max_size).
    pub fn max_size(mut self, n: u32) -> Self {
        self.max_size = n;
        self
    }

    /// Sets [`acquire_timeout`](Self::acquire_timeout).
    pub fn acquire_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.acquire_timeout = timeout;
        self
    }

    /// Sets [`idle_timeout`](Self::idle_timeout).
    pub fn idle_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Sets [`max_lifetime`](Self::max_lifetime).
    pub fn max_lifetime(mut self, lifetime: Option<Duration>) -> Self {
        self.max_lifetime = lifetime;
        self
    }
}

// ---------------------------------------------------------------------------
// Durations
// ---------------------------------------------------------------------------

/// Parses a duration: a number of milliseconds, or a number with a unit
/// (`ms`, `s`, `m`, `h`), e.g. `"250ms"`, `"30s"`, `"1.5s"`, `"10m"`.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (number, unit) = s.split_at(split);
    let number: f64 = number
        .parse()
        .map_err(|_| format!("invalid duration `{s}`"))?;
    let seconds = match unit.trim() {
        "" | "ms" => number / 1000.0,
        "s" => number,
        "m" | "min" => number * 60.0,
        "h" => number * 3600.0,
        other => return Err(format!("unknown duration unit `{other}` in `{s}`")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|e| format!("invalid duration `{s}`: {e}"))
}

/// Formats a duration the way [`parse_duration`] reads it.
pub fn format_duration(d: Duration) -> String {
    match d.as_millis() {
        _ if !d.subsec_nanos().is_multiple_of(1_000_000) => format!("{}s", d.as_secs_f64()),
        ms if ms % 3_600_000 == 0 && ms > 0 => format!("{}h", ms / 3_600_000),
        ms if ms % 60_000 == 0 && ms > 0 => format!("{}m", ms / 60_000),
        ms if ms % 1000 == 0 && ms > 0 => format!("{}s", ms / 1000),
        ms => format!("{ms}ms"),
    }
}

/// Serde support for `Option<Duration>` as `"30s"` / milliseconds / `"off"` / null.
mod duration {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(d) => s.serialize_str(&super::format_duration(*d)),
            None => s.serialize_none(),
        }
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Millis(u64),
        Text(String),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        Ok(match Option::<Repr>::deserialize(d)? {
            None => None,
            Some(Repr::Millis(ms)) => Some(Duration::from_millis(ms)),
            Some(Repr::Text(s)) if matches!(s.trim(), "off" | "none" | "never") => None,
            Some(Repr::Text(s)) => Some(super::parse_duration(&s).map_err(de::Error::custom)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(v: serde_json::Value) -> Result<DbConfig, String> {
        serde_json::from_value(v).map_err(|e| e.to_string())
    }

    #[test]
    fn schemes() {
        assert_eq!(
            DbConfig::new("postgres://h/db").scheme().as_deref(),
            Some("postgres")
        );
        assert_eq!(
            DbConfig::new("sqlite::memory:").scheme().as_deref(),
            Some("sqlite")
        );
        assert_eq!(
            DbConfig::new("SQLite://x.db").scheme().as_deref(),
            Some("sqlite")
        );
        assert_eq!(DbConfig::new("memory:").scheme().as_deref(), Some("memory"));
        assert_eq!(DbConfig::new("no scheme").scheme(), None);
    }

    #[test]
    fn passwords_are_redacted() {
        let c = DbConfig::new("postgres://app:s3cr@t@db:5432/app?sslmode=require");
        assert_eq!(
            c.redacted_url(),
            "postgres://app:***@db:5432/app?sslmode=require"
        );
        assert!(!format!("{c:?}").contains("s3cr"));
        for unchanged in [
            "postgres://app@db/x",
            "sqlite://data/a.db",
            "sqlite::memory:",
        ] {
            assert_eq!(redact(unchanged), unchanged);
        }
    }

    #[test]
    fn durations() {
        for (text, ms) in [
            ("250ms", 250),
            ("30s", 30_000),
            ("1.5s", 1500),
            ("2m", 120_000),
            ("1h", 3_600_000),
            ("40", 40),
        ] {
            let d = Duration::from_millis(ms);
            assert_eq!(parse_duration(text), Ok(d), "{text}");
            assert_eq!(parse_duration(&format_duration(d)), Ok(d));
        }
        assert!(parse_duration("5 parsecs").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn shared_fields_and_pool() {
        let c = parse(json!({
            "url": "sqlite::memory:",
            "qualifier": "primary",
            "pool": { "max_size": 4, "acquire_timeout": "5s", "idle_timeout": null, "max_lifetime": "off" },
        }))
        .unwrap();
        assert_eq!(c.qualifier.as_deref(), Some("primary"));
        assert_eq!(c.pool.max_size, 4);
        assert_eq!(c.pool.acquire_timeout, Some(Duration::from_secs(5)));
        assert_eq!(c.pool.idle_timeout, None);
        assert_eq!(c.pool.max_lifetime, None);
        assert_eq!(c.driver, DriverConfig::Auto, "no tag, no options");
        assert!(parse(json!({ "url": "x:", "pool": { "max": 1 } })).is_err());
    }

    #[test]
    fn tagged_driver_options_are_typed() {
        let c = parse(json!({
            "url": "sqlite://app.db",
            "driver": "sqlite",
            "busy_timeout": "1s",
            "journal_mode": "wal",
        }))
        .unwrap();
        assert_eq!(
            c.driver,
            DriverConfig::Sqlite(SqliteOptions {
                busy_timeout: Some(Duration::from_secs(1)),
                foreign_keys: true,
                journal_mode: Some("wal".into()),
                statement_cache_size: None,
            })
        );
        let err = parse(json!({ "url": "sqlite://a.db", "driver": "sqlite", "busy": "1s" }));
        assert!(
            err.unwrap_err().contains("busy"),
            "unknown options are refused"
        );
        let err = parse(json!({ "url": "memory:", "driver": "memory", "x": 1 }));
        assert!(err.is_err());
    }

    #[test]
    fn untagged_options_infer_the_driver_from_the_scheme() {
        let c = parse(json!({ "url": "postgres://h/db", "application_name": "api" })).unwrap();
        assert_eq!(
            c.driver,
            DriverConfig::Postgres(PostgresOptions {
                application_name: Some("api".into()),
                connect_timeout: None,
                statement_cache_size: None,
            })
        );
        assert!(parse(json!({ "url": "foo://h", "x": 1 })).is_err());
    }

    #[test]
    fn unknown_drivers_keep_their_options_as_strings() {
        let c = parse(json!({ "url": "duck://x", "driver": "duckdb", "threads": 4, "mode": "ro" }))
            .unwrap();
        let DriverConfig::Other { name, options } = &c.driver else {
            panic!("{c:?}")
        };
        assert_eq!(name, "duckdb");
        assert_eq!(options["threads"], "4");
        assert_eq!(options["mode"], "ro");
    }

    #[test]
    fn round_trips() {
        for c in [
            DbConfig::new("sqlite::memory:"),
            DbConfig::new("mysql://h/db")
                .qualifier("replica")
                .driver(MySqlOptions {
                    statement_cache_size: Some(64),
                    init: vec!["SET time_zone = '+00:00'".into()],
                }),
            DbConfig::new("postgres://h/db").driver(DriverConfig::PostgresSync(PostgresOptions {
                application_name: None,
                connect_timeout: Some(Duration::from_secs(3)),
                statement_cache_size: Some(0),
            })),
            DbConfig::new("memory:").driver(DriverConfig::Memory),
        ] {
            let value = serde_json::to_value(&c).unwrap();
            assert_eq!(parse(value.clone()).unwrap(), c, "{value}");
        }
    }
}
