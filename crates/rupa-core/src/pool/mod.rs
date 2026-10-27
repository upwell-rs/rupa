//! Connection pools, as seen by rupa.
//!
//! rupa doesn't implement pooling itself: each driver crate wraps an
//! established pool (`r2d2` for sync drivers, `deadpool` for async ones)
//! behind its `pool` cargo feature, and implements [`Pool`] or [`AsyncPool`]
//! for it. In return the pool gets, from here:
//!
//! - [`Acquire`] / [`AcquireAsync`](crate::repo::AcquireAsync), so repository views run queries through the pool
//!   repositories with one pooled connection per call;
//! - **owned transactions** (`pool.begin().await?`): an [`AsyncTx`](crate::tx::AsyncTx) that holds
//!   its pooled connection, so it is `'static` and can live in a request
//!   scope. Dropping one returns the connection to the pool;
//! - `begin_with_security`, statically gated like the executor version;
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
#[cfg(all(feature = "deadpool", feature = "async"))]
pub mod deadpool;
#[cfg(feature = "r2d2")]
pub mod r2d2;

use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::dialect::{Supports, caps};
use crate::exec::{Engine, Executor, Outcome};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;
use crate::repo::Acquire;
use crate::security::{self, SecureTx, SecurityContext};
use crate::tx::{Tx, TxConnection, TxOptions};

#[cfg(feature = "async")]
pub use asynchronous::{AsyncPool, OwnedAsyncTx, SingleConnection};

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

/// A sync transaction that owns its pooled connection; see [`Pool::begin`].
pub type OwnedTx<C> = Tx<Box<C>>;

type ConnError<C> = <C as Executor>::Error;

/// A pool of sync connections.
///
/// Implementations hand out owned connections (`'static`), which return to
/// the pool when dropped. Pool failures (e.g. a timeout waiting for a
/// connection) are reported as the connection's error type.
pub trait Pool: Send + Sync + 'static {
    /// A pooled connection; usually [`Pooled`]`<the pool's guard>`.
    type Conn: TxConnection + 'static;

    /// The connections' dialect.
    fn dialect(&self) -> &<Self::Conn as Executor>::Dialect;

    /// Checks a connection out, waiting up to the pool's acquire timeout.
    fn get(&self) -> Result<Self::Conn, ConnError<Self::Conn>>;

    /// The pool's current occupancy.
    fn status(&self) -> PoolStatus;

    /// Opens a transaction on a pooled connection it owns. The connection
    /// returns to the pool when the transaction ends or is dropped.
    fn begin_with(&self, options: TxOptions) -> Result<OwnedTx<Self::Conn>, ConnError<Self::Conn>> {
        Tx::begin(Box::new(self.get()?), options)
    }

    /// [`begin_with`](Self::begin_with) default options.
    fn begin(&self) -> Result<OwnedTx<Self::Conn>, ConnError<Self::Conn>> {
        self.begin_with(TxOptions::default())
    }

    /// Opens an owned transaction with `context` applied (see
    /// [`crate::security`]). Only compiles for dialects with native
    /// row-level security.
    fn begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<OwnedTx<Self::Conn>>, ConnError<Self::Conn>>
    where
        <Self::Conn as Executor>::Dialect: Supports<caps::NativeRls>,
    {
        security::apply(self.begin_with(options)?, context)
    }
}

impl<P: Pool> Acquire for P {
    type Dialect = <P::Conn as Executor>::Dialect;
    type Error = ConnError<P::Conn>;
    type Conn<'a> = P::Conn;

    fn acquire(&self) -> Result<P::Conn, Self::Error> {
        self.get()
    }
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
impl<G: DerefMut<Target: Engine>> Engine for Pooled<G> {
    type Completion = <G::Target as Engine>::Completion;
}

impl<G: DerefMut<Target: Executor>> Executor for Pooled<G> {
    type Dialect = <G::Target as Executor>::Dialect;
    type Error = <G::Target as Executor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.0.dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error> {
        self.0.execute(statement, expect)
    }
}

impl<G: DerefMut<Target: TxConnection> + Send> TxConnection for Pooled<G> {
    fn control(&mut self, statement: TxStatement) -> Result<(), Self::Error> {
        self.0.control(statement)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.0.defer_rollback(depth);
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&mut self) -> Result<(), Self::Error> {
        self.0.clean()
    }
}
