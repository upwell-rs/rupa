//! The typed options of rupa's own drivers.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::duration;

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
