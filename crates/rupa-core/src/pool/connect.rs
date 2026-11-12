//! Opening a pool, or one connection outside it, from a [`DbConfig`].

#[cfg(feature = "async")]
use std::future::Future;

use crate::config::DbConfig;
use crate::error::ConnectError;
#[cfg(feature = "async")]
use crate::exec::Awaited;
use crate::exec::Immediate;

/// A pool that opens itself, and single connections outside itself, from a
/// [`DbConfig`].
///
/// A single connection serves work that must not share the pool's
/// connections or credentials, such as a migration run. It is opened with
/// the same driver options, TLS and URL handling as the pool's own.
/// `Completion` says how: [`Immediate`] for [`SyncConnect`], `Awaited` for
/// `AsyncConnect`.
pub trait Connect: Clone + Send + Sync + Sized + 'static {
    /// [`Immediate`] for a sync pool, `Awaited` for an async one.
    type Completion;
    /// A connection opened outside the pool.
    type Connection: Send + 'static;
}

/// A sync pool's [`Connect`].
pub trait SyncConnect: Connect<Completion = Immediate> {
    /// Opens the pool `config` describes.
    fn connect(config: &DbConfig) -> Result<Self, ConnectError>;

    /// Opens one connection outside any pool, as the pool would.
    fn connect_one(config: &DbConfig) -> Result<Self::Connection, ConnectError>;
}

/// An async pool's [`Connect`].
#[cfg(feature = "async")]
pub trait AsyncConnect: Connect<Completion = Awaited> {
    /// Opens the pool `config` describes.
    fn connect(config: &DbConfig) -> impl Future<Output = Result<Self, ConnectError>> + Send;

    /// Opens one connection outside any pool, as the pool would.
    fn connect_one(
        config: &DbConfig,
    ) -> impl Future<Output = Result<Self::Connection, ConnectError>> + Send;
}
