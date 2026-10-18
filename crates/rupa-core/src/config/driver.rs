//! [`DriverConfig`]: which driver, with its typed options.

use std::collections::BTreeMap;

use super::{MySqlOptions, PostgresOptions, SqliteOptions};
use crate::DialectId;
use crate::error::ConnectError;

/// Which driver to use, with its options. See the [module docs](crate::config).
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
    ///
    /// Async: a sync driver blocks the runtime for each round trip.
    pub fn for_scheme(scheme: &str) -> Option<Self> {
        Some(match scheme {
            "postgres" | "postgresql" => DriverConfig::Postgres(PostgresOptions::default()),
            "sqlite" => DriverConfig::Sqlite(SqliteOptions::default()),
            "mysql" => DriverConfig::SqlxMySql(MySqlOptions::default()),
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
