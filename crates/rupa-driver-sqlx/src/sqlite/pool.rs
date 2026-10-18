//! The sqlx-sqlite pool.

use rupa_core::config::{DbConfig, OpenMode, SqliteOptions, SqliteTarget};
use rupa_core::error::ConnectError;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use std::str::FromStr;

use crate::PoolBackend;
use crate::pool::SqlxManager;

/// A pool of SQLite connections.
pub type SqlitePool = crate::SqlxPool<sqlx::Sqlite>;
/// Opens and recycles [`SqliteExecutor`](super::SqliteExecutor)s for a [`SqlitePool`].
pub type SqliteManager = crate::SqlxManager<sqlx::Sqlite>;
/// The registry driver `sqlx-sqlite`.
#[cfg(feature = "dynamic")]
pub type SqliteDriver = crate::SqlxDriver<sqlx::Sqlite>;

impl PoolBackend for sqlx::Sqlite {
    /// Each connection to `:memory:` would be its own database.
    fn manager(config: &DbConfig) -> Result<SqlxManager<Self>, ConnectError> {
        let SqliteOptions {
            busy_timeout,
            foreign_keys,
            journal_mode,
            statement_cache_size,
        } = config.driver.sqlite()?;
        let target = SqliteTarget::parse(&config.url).map_err(ConnectError::InvalidUrl)?;
        let mut options = match &target {
            SqliteTarget::Memory => SqliteConnectOptions::from_str("sqlite::memory:")
                .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?,
            SqliteTarget::File { path, mode } => SqliteConnectOptions::new()
                .filename(path)
                .read_only(*mode == OpenMode::ReadOnly)
                .create_if_missing(*mode == OpenMode::ReadWriteCreate),
        };
        options = options
            .busy_timeout(busy_timeout.unwrap_or_default())
            .foreign_keys(foreign_keys);
        if let Some(mode) = &journal_mode {
            let mode =
                SqliteJournalMode::from_str(mode).map_err(|e| ConnectError::InvalidOption {
                    key: "journal_mode".into(),
                    message: e.to_string(),
                })?;
            options = options.journal_mode(mode);
        }

        if let Some(capacity) = statement_cache_size {
            options = options.statement_cache_capacity(capacity);
        }

        let manager = SqlxManager::new(options);
        Ok(match target {
            SqliteTarget::Memory => manager.max_connections(Some(1)),
            SqliteTarget::File { .. } => manager,
        })
    }
}
