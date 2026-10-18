//! [`PoolConfig`]: pool sizing and timeouts, shared by every driver.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::duration;

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
