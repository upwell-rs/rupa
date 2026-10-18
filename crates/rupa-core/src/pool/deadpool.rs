//! Glue for async driver pools built on `deadpool` (on tokio).

use std::error::Error as StdError;
use std::time::Duration;

use deadpool::managed::{Manager, Metrics, Object, Pool, PoolBuilder, PoolError, RecycleError};

use super::{PoolStatus, Pooled};
use crate::config::PoolConfig;

/// A `deadpool` builder for at most `max_size` connections, with `config`'s
/// timeouts, on tokio.
pub fn builder<M: Manager>(manager: M, config: &PoolConfig, max_size: u32) -> PoolBuilder<M> {
    Pool::builder(manager)
        .max_size(max_size as usize)
        .wait_timeout(config.acquire_timeout)
        .create_timeout(config.acquire_timeout)
        .runtime(deadpool::Runtime::Tokio1)
}

/// Checks a connection out of `pool`. The manager's own errors are returned
/// as they are; pool failures (timeouts, a closed pool) are wrapped by `wrap`.
pub async fn get<M, E>(
    pool: &Pool<M>,
    wrap: impl FnOnce(Box<dyn StdError + Send + Sync>) -> E,
) -> Result<Pooled<Object<M>>, E>
where
    M: Manager<Error = E>,
    E: StdError + Send + Sync + 'static,
{
    match pool.get().await {
        Ok(conn) => Ok(Pooled::new(conn)),
        Err(PoolError::Backend(e)) => Err(e),
        Err(e) => Err(wrap(Box::new(e))),
    }
}

/// The occupancy of `pool`.
pub fn status<M: Manager>(pool: &Pool<M>) -> PoolStatus {
    let status = pool.status();
    let as_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);

    PoolStatus {
        max_size: as_u32(status.max_size),
        size: as_u32(status.size),
        idle: as_u32(status.available),
    }
}

/// When a recycled connection is too old or was idle too long to keep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Expiry {
    /// Discard connections idle for longer than this.
    pub idle_timeout: Option<Duration>,
    /// Discard connections older than this.
    pub max_lifetime: Option<Duration>,
}

impl Expiry {
    /// `config`'s idle timeout and maximum lifetime.
    pub fn from_config(config: &PoolConfig) -> Self {
        Self {
            idle_timeout: config.idle_timeout,
            max_lifetime: config.max_lifetime,
        }
    }

    /// `Err` if a connection with `metrics` must be discarded.
    pub fn check<E>(&self, metrics: &Metrics) -> Result<(), RecycleError<E>> {
        if self.max_lifetime.is_some_and(|max| metrics.age() > max) {
            return Err(RecycleError::message("connection reached max_lifetime"));
        }

        if self
            .idle_timeout
            .is_some_and(|max| metrics.last_used() > max)
        {
            return Err(RecycleError::message(
                "connection idle for longer than idle_timeout",
            ));
        }

        Ok(())
    }
}
