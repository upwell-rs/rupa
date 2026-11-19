//! Connection pools, as seen by rupa.
//!
//! rupa doesn't implement pooling itself: each driver crate wraps an
//! established pool (`r2d2` for sync drivers, `deadpool` for async ones)
//! behind its `pool` cargo feature, and implements [`Pool`] or [`AsyncPool`]
//! for it, plus the executor traits through [`pool_executor!`](crate::pool_executor).
//! A pool is then a [`Connection`](crate::connection::Connection) like any
//! other:
//!
//! - every statement runs on a connection checked out for it, so
//!   `query.execute(&pool)` and repositories over the pool need no lock;
//! - **owned transactions** (`pool.begin()?`): a [`Tx`] that holds its
//!   pooled connection, so it is `'static` and can live in a request scope.
//!   Dropping one returns the connection to the pool;
//! - `begin_with_security`, statically gated like every transactional
//!   executor's;
//! - type erasure, through [`Database::new`](crate::registry::Database::new).
//!
//! # Dirty connections
//!
//! A connection whose async transaction was dropped while open owes a
//! rollback ([`AsyncTxConnection::is_dirty`](crate::tx::AsyncTxConnection::is_dirty)). Drivers' pools
//! [`clean`](crate::tx::AsyncTxConnection::clean) such connections when they are
//! recycled, and discard them if that fails. Even without that check, the
//! driver sends the rollback before the connection's next statement.
//!
//! # Sync pools in async code
//!
//! [`Blocking<P>`](crate::exec::Blocking) turns a sync [`Pool`] into an [`AsyncPool`]
//! whose futures complete immediately (the calling thread blocks while
//! waiting for a connection and while statements run), as [`Blocking<E>`](crate::exec::Blocking)
//! does for one executor.

#[cfg(feature = "async")]
mod asynchronous;
mod connect;
#[cfg(all(feature = "deadpool", feature = "async"))]
pub mod deadpool;
#[cfg(feature = "r2d2")]
pub mod r2d2;

use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::error::TxError;
use crate::exec::{Engine, Executor, Outcome};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;
use crate::tx::{Transactional, Tx, TxConnection, TxOptions, TxSlot};

#[cfg(feature = "async")]
pub use asynchronous::{AsyncPool, OwnedAsyncTx, SingleConnection};
#[cfg(feature = "async")]
#[doc(hidden)]
pub use asynchronous::{begin_async, execute_async};
#[cfg(feature = "async")]
pub use connect::AsyncConnect;
pub use connect::{Connect, SyncConnect};

/// A snapshot of a pool's occupancy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PoolStatus {
    /// The most connections the pool will open.
    pub max_size: u32,
    /// Connections currently open (idle or in use).
    pub size: u32,
    /// Open connections not in use.
    pub idle: u32,
}

impl PoolStatus {
    /// Connections currently checked out.
    pub fn in_use(&self) -> u32 {
        self.size.saturating_sub(self.idle)
    }
}

/// A sync transaction that owns its pooled connection; what `begin` on a
/// [`Pool`] returns.
pub type OwnedTx<C> = Tx<Box<C>>;

/// A pool of sync connections.
///
/// Implementations hand out owned connections (`'static`), which return to
/// the pool when dropped. Pool failures (e.g. a timeout waiting for a
/// connection) are reported as the connection's error type. A pool is an
/// executor itself, implemented with [`pool_executor!`](crate::pool_executor):
/// each statement runs on a connection checked out for it, and `begin` opens
/// an [`OwnedTx`].
pub trait Pool: Transactional<Error: From<TxError>> + Send + Sync + 'static {
    /// A pooled connection; usually [`Pooled`]`<the pool's guard>`.
    type Conn: TxConnection<Dialect = Self::Dialect, Error = Self::Error>
        + Transactional
        + Sync
        + 'static;

    /// Checks a connection out, waiting up to the pool's acquire timeout.
    fn get(&self) -> Result<Self::Conn, Self::Error>;

    /// The pool's current occupancy.
    fn status(&self) -> PoolStatus;
}

/// Runs `statement` on a connection checked out of `pool`; how a pool is an
/// executor. Not public API.
#[doc(hidden)]
pub fn execute<P: Pool>(
    pool: &P,
    statement: &Statement,
    expect: Expect,
) -> Result<Outcome, P::Error> {
    pool.get()?.execute(statement, expect)
}

/// Opens a transaction that owns a connection checked out of `pool`. Not
/// public API.
#[doc(hidden)]
pub fn begin<P: Pool>(pool: &P, options: TxOptions) -> Result<OwnedTx<P::Conn>, P::Error> {
    Tx::begin(Box::new(pool.get()?), options)
}

/// Implements the executor traits of a pool type through its [`Pool`] or
/// [`AsyncPool`] implementation: statements run on a checked-out connection,
/// transactions own one. The pool type holds its dialect at `$dialect`.
///
/// ```ignore
/// rupa_core::pool_executor!(sync, [] SqlitePool, |pool| &pool.dialect);
/// rupa_core::pool_executor!(async, [DB: Backend] SqlxPool<DB>, |pool| &pool.dialect);
/// ```
#[macro_export]
macro_rules! pool_executor {
    (sync, [$($generics:tt)*] $pool:ty, |$this:ident| $dialect:expr) => {
        impl<$($generics)*> $crate::exec::Engine for $pool {
            type Completion = $crate::exec::Immediate;
        }

        impl<$($generics)*> $crate::exec::Executor for $pool {
            type Dialect = <<$pool as $crate::pool::Pool>::Conn as $crate::exec::Executor>::Dialect;
            type Error = <<$pool as $crate::pool::Pool>::Conn as $crate::exec::Executor>::Error;

            fn dialect(&self) -> &Self::Dialect {
                let $this = self;

                $dialect
            }

            fn execute(
                &self,
                statement: &$crate::ir::Statement,
                expect: $crate::query::Expect,
            ) -> ::core::result::Result<$crate::exec::Outcome, Self::Error> {
                $crate::pool::execute(self, statement, expect)
            }
        }

        impl<$($generics)*> $crate::tx::Transactional for $pool {
            type Tx<'t> = $crate::pool::OwnedTx<<$pool as $crate::pool::Pool>::Conn>;

            fn begin_with(
                &self,
                options: $crate::tx::TxOptions,
            ) -> ::core::result::Result<Self::Tx<'_>, Self::Error> {
                $crate::pool::begin(self, options)
            }
        }
    };
    (async, [$($generics:tt)*] $pool:ty, |$this:ident| $dialect:expr) => {
        impl<$($generics)*> $crate::exec::Engine for $pool {
            type Completion = $crate::exec::Awaited;
        }

        impl<$($generics)*> $crate::exec::AsyncExecutor for $pool {
            type Dialect = <<$pool as $crate::pool::AsyncPool>::Conn as $crate::exec::AsyncExecutor>::Dialect;
            type Error = <<$pool as $crate::pool::AsyncPool>::Conn as $crate::exec::AsyncExecutor>::Error;

            fn dialect(&self) -> &Self::Dialect {
                let $this = self;

                $dialect
            }

            fn execute(
                &self,
                statement: &$crate::ir::Statement,
                expect: $crate::query::Expect,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<$crate::exec::Outcome, Self::Error>,
            > + ::core::marker::Send {
                $crate::pool::execute_async(self, statement, expect)
            }
        }

        impl<$($generics)*> $crate::tx::AsyncTransactional for $pool {
            type Tx<'t> = $crate::pool::OwnedAsyncTx<<$pool as $crate::pool::AsyncPool>::Conn>;

            fn begin_with(
                &self,
                options: $crate::tx::TxOptions,
            ) -> impl ::core::future::Future<
                Output = ::core::result::Result<Self::Tx<'_>, Self::Error>,
            > + ::core::marker::Send {
                $crate::pool::begin_async(self, options)
            }
        }
    };
}

/// A connection checked out of a pool: the pool's guard type `G` (which
/// derefs to the driver's executor), usable as that executor.
///
/// Forwards [`Executor`] / [`AsyncExecutor`](crate::exec::AsyncExecutor) and the transaction traits, so
/// it is transactional exactly when the driver's connection is.
pub struct Pooled<G>(G);

impl<G> Pooled<G> {
    /// Wraps a pool guard.
    pub fn new(guard: G) -> Self {
        Self(guard)
    }

    /// The pool guard.
    pub fn into_inner(self) -> G {
        self.0
    }
}

impl<G: Deref> Deref for Pooled<G> {
    type Target = G::Target;
    fn deref(&self) -> &G::Target {
        &self.0
    }
}

impl<G: DerefMut> DerefMut for Pooled<G> {
    fn deref_mut(&mut self) -> &mut G::Target {
        &mut self.0
    }
}

impl<G: Deref<Target: fmt::Debug>> fmt::Debug for Pooled<G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Pooled").field(&&*self.0).finish()
    }
}

/// A pooled connection runs queries in its connection's mode.
impl<G: Deref<Target: Engine>> Engine for Pooled<G> {
    type Completion = <G::Target as Engine>::Completion;
}

impl<G: Deref<Target: Executor>> Executor for Pooled<G> {
    type Dialect = <G::Target as Executor>::Dialect;
    type Error = <G::Target as Executor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.0.dialect()
    }

    fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error> {
        self.0.execute(statement, expect)
    }
}

impl<G: Deref<Target: TxConnection> + Send> TxConnection for Pooled<G> {
    fn control(&self, statement: TxStatement) -> Result<(), Self::Error> {
        self.0.control(statement)
    }

    fn defer_rollback(&self, depth: u32) {
        self.0.defer_rollback(depth);
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&self) -> Result<(), Self::Error> {
        self.0.clean()
    }

    fn slot(&self) -> &TxSlot {
        self.0.slot()
    }
}

impl<G: Deref<Target: TxConnection> + Send + Sync> Transactional for Pooled<G> {
    type Tx<'t>
        = Tx<&'t Self>
    where
        Self: 't;

    fn begin_with(&self, options: TxOptions) -> Result<Tx<&Self>, Self::Error> {
        Tx::begin(self, options)
    }
}
