//! Glue for driver pools built on `r2d2`.

use std::time::Duration;

use super::PoolStatus;
use crate::config::PoolConfig;

#[doc(hidden)]
pub use r2d2 as __r2d2;

/// r2d2 has no "wait forever"; a century will do.
const FOREVER: Duration = Duration::from_secs(100 * 365 * 24 * 3600);

/// An `r2d2` builder with `config`'s sizing and timeouts. An unset or zero
/// acquire timeout waits (practically) forever.
pub fn builder<M: r2d2::ManageConnection>(config: &PoolConfig) -> r2d2::Builder<M> {
    let acquire_timeout = config.acquire_timeout.filter(|t| !t.is_zero());

    r2d2::Pool::builder()
        .max_size(config.max_size)
        .min_idle(Some(config.min_idle.unwrap_or(1).min(config.max_size)))
        .idle_timeout(config.idle_timeout)
        .max_lifetime(config.max_lifetime)
        .connection_timeout(acquire_timeout.unwrap_or(FOREVER))
}

/// The occupancy of `pool`.
pub fn status<M: r2d2::ManageConnection>(pool: &r2d2::Pool<M>) -> PoolStatus {
    let state = pool.state();

    PoolStatus {
        max_size: pool.max_size(),
        size: state.connections,
        idle: state.idle_connections,
    }
}

/// Declares a sync driver's pool: a newtype over `r2d2::Pool<manager>` that
/// implements [`Pool`](crate::pool::Pool), with `from_r2d2`, `r2d2` and a
/// private `build(manager, builder)` that opens one connection first, so a
/// wrong URL or password fails with the driver's own error (r2d2 would retry
/// until the acquire timeout and report only that).
///
/// `error` must have a `Pool(Box<dyn Error + Send + Sync>)` variant, as
/// `rupa_sql::driver_error!` declares.
#[macro_export]
#[doc(hidden)]
macro_rules! r2d2_pool {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            manager: $manager:ty,
            dialect: $dialect:ident,
            error: $error:ident $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone)]
        $vis struct $name {
            pool: $crate::pool::r2d2::__r2d2::Pool<$manager>,
            dialect: $dialect,
        }

        impl $name {
            /// Wraps an `r2d2` pool built elsewhere.
            pub fn from_r2d2(pool: $crate::pool::r2d2::__r2d2::Pool<$manager>) -> Self {
                Self {
                    pool,
                    dialect: $dialect,
                }
            }

            /// The underlying `r2d2` pool.
            pub fn r2d2(&self) -> &$crate::pool::r2d2::__r2d2::Pool<$manager> {
                &self.pool
            }

            fn build(
                manager: $manager,
                builder: $crate::pool::r2d2::__r2d2::Builder<$manager>,
            ) -> ::std::result::Result<Self, $error> {
                ::std::mem::drop($crate::pool::r2d2::__r2d2::ManageConnection::connect(&manager)?);

                builder
                    .build(manager)
                    .map(Self::from_r2d2)
                    .map_err(|e| $error::Pool(::std::boxed::Box::new(e)))
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(::std::stringify!($name))
                    .field("status", &$crate::pool::Pool::status(self))
                    .finish_non_exhaustive()
            }
        }

        impl $crate::pool::Pool for $name {
            type Conn = $crate::pool::Pooled<$crate::pool::r2d2::__r2d2::PooledConnection<$manager>>;

            fn dialect(&self) -> &$dialect {
                &self.dialect
            }

            fn get(&self) -> ::std::result::Result<Self::Conn, $error> {
                self.pool
                    .get()
                    .map($crate::pool::Pooled::new)
                    .map_err(|e| $error::Pool(::std::boxed::Box::new(e)))
            }

            fn status(&self) -> $crate::pool::PoolStatus {
                $crate::pool::r2d2::status(&self.pool)
            }
        }
    };
}
