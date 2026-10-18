//! The sqlx-postgres pool.

use rupa_core::config::{DbConfig, PostgresOptions};
use rupa_core::error::ConnectError;
use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;

use crate::PoolBackend;
use crate::pool::SqlxManager;

/// A pool of Postgres connections.
pub type PgPool = crate::SqlxPool<sqlx::Postgres>;
/// Opens and recycles [`PgExecutor`](super::PgExecutor)s for a [`PgPool`].
pub type PgManager = crate::SqlxManager<sqlx::Postgres>;
/// The registry driver `sqlx-postgres`.
#[cfg(feature = "dynamic")]
pub type PgDriver = crate::SqlxDriver<sqlx::Postgres>;

impl PoolBackend for sqlx::Postgres {
    fn manager(config: &DbConfig) -> Result<SqlxManager<Self>, ConnectError> {
        let PostgresOptions {
            application_name,
            connect_timeout,
            statement_cache_size,
        } = config.driver.postgres()?;
        let mut options = PgConnectOptions::from_str(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        if let Some(name) = &application_name {
            options = options.application_name(name);
        }

        if let Some(capacity) = statement_cache_size {
            options = options.statement_cache_capacity(capacity);
        }

        Ok(SqlxManager::new(options).connect_timeout(connect_timeout))
    }
}
