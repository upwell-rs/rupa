//! The sqlx-mysql pool.

use rupa_core::config::{DbConfig, MySqlOptions};
use rupa_core::error::ConnectError;
use sqlx::mysql::MySqlConnectOptions;
use std::str::FromStr;

use crate::PoolBackend;
use crate::pool::SqlxManager;

/// A pool of MySQL connections.
pub type MySqlPool = crate::SqlxPool<sqlx::MySql>;
/// Opens and recycles [`MySqlExecutor`](super::MySqlExecutor)s for a [`MySqlPool`].
pub type MySqlManager = crate::SqlxManager<sqlx::MySql>;
/// The registry driver `sqlx-mysql`.
#[cfg(feature = "dynamic")]
pub type MySqlDriver = crate::SqlxDriver<sqlx::MySql>;

impl PoolBackend for sqlx::MySql {
    fn manager(config: &DbConfig) -> Result<SqlxManager<Self>, ConnectError> {
        let MySqlOptions {
            statement_cache_size,
            init,
        } = config.driver.mysql()?;
        let mut options = MySqlConnectOptions::from_str(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        if let Some(capacity) = statement_cache_size {
            options = options.statement_cache_capacity(capacity);
        }

        Ok(SqlxManager::new(options).init(init))
    }
}
