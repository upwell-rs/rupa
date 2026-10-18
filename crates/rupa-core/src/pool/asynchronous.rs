//! Async pools, sync pools used from async code, and one connection as a
//! pool.

use std::fmt;
use std::future::Future;
use std::ops::DerefMut;
use std::sync::Arc;

use super::{ConnError, Pool, PoolStatus, Pooled};
use crate::dialect::{Supports, caps};
use crate::exec::{AsyncExecutor, Blocking, Executor, Outcome};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;
use crate::repo::AcquireAsync;
use crate::security::{SecureTx, SecurityContext, apply_security};
use crate::tx::{AsyncTx, AsyncTxConnection, TxOptions};

/// An async transaction that owns its pooled connection; see
/// [`AsyncPool::begin`].
pub type OwnedAsyncTx<C> = AsyncTx<Box<C>>;

type AsyncConnError<C> = <C as AsyncExecutor>::Error;

/// A pool of async connections; see [`Pool`].
pub trait AsyncPool: Send + Sync + 'static {
    /// A pooled connection; usually [`Pooled`]`<the pool's guard>`.
    type Conn: AsyncTxConnection + 'static;

    /// The connections' dialect.
    fn dialect(&self) -> &<Self::Conn as AsyncExecutor>::Dialect;

    /// Checks a connection out, waiting up to the pool's acquire timeout.
    fn get(&self) -> impl Future<Output = Result<Self::Conn, AsyncConnError<Self::Conn>>> + Send;

    /// The pool's current occupancy.
    fn status(&self) -> PoolStatus;

    /// Opens a transaction on a pooled connection it owns. The connection
    /// returns to the pool when the transaction ends or is dropped (dirty, if
    /// it was dropped while open; the pool cleans it).
    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<OwnedAsyncTx<Self::Conn>, AsyncConnError<Self::Conn>>> + Send
    {
        async move { AsyncTx::begin(Box::new(self.get().await?), options).await }
    }

    /// [`begin_with`](Self::begin_with) default options.
    fn begin(
        &self,
    ) -> impl Future<Output = Result<OwnedAsyncTx<Self::Conn>, AsyncConnError<Self::Conn>>> + Send
    {
        self.begin_with(TxOptions::default())
    }

    /// Opens an owned transaction with `context` applied (see
    /// [`crate::security`]). Only compiles for dialects with native
    /// row-level security.
    fn begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<OwnedAsyncTx<Self::Conn>>, AsyncConnError<Self::Conn>>>
    + Send
    where
        <Self::Conn as AsyncExecutor>::Dialect: Supports<caps::NativeRls>,
    {
        async move { apply_security(self.begin_with(options).await?, context).await }
    }
}

impl<P: AsyncPool> AcquireAsync for P {
    type Dialect = <P::Conn as AsyncExecutor>::Dialect;
    type Error = AsyncConnError<P::Conn>;
    type Conn<'a> = P::Conn;

    fn acquire(&self) -> impl Future<Output = Result<P::Conn, Self::Error>> + Send {
        self.get()
    }
}

/// A sync pool used from async code; see the [module docs](crate::pool).
impl<P: Pool> AsyncPool for Blocking<P> {
    type Conn = Blocking<P::Conn>;

    fn dialect(&self) -> &<P::Conn as Executor>::Dialect {
        self.0.dialect()
    }

    fn get(&self) -> impl Future<Output = Result<Blocking<P::Conn>, ConnError<P::Conn>>> + Send {
        std::future::ready(self.0.get().map(Blocking))
    }

    fn status(&self) -> PoolStatus {
        self.0.status()
    }
}

impl<G: DerefMut<Target: AsyncExecutor> + Send> AsyncExecutor for Pooled<G> {
    type Dialect = <G::Target as AsyncExecutor>::Dialect;
    type Error = <G::Target as AsyncExecutor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.0.dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        self.0.execute(statement, expect)
    }
}

impl<G: DerefMut<Target: AsyncTxConnection> + Send> AsyncTxConnection for Pooled<G> {
    fn control(
        &mut self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.0.control(statement)
    }

    fn defer_rollback(&mut self, depth: u32) {
        self.0.defer_rollback(depth);
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.0.clean()
    }
}

/// One async connection as an [`AsyncPool`] of size one: `get` waits until
/// the previous user dropped it.
///
/// For backends where a pool makes no sense (the in-memory database) or for
/// tests. Unlike [`SharedAsync`](crate::repo::SharedAsync), connections are
/// owned, so it supports owned transactions and [`Database`](crate::registry::Database).
pub struct SingleConnection<E: AsyncExecutor> {
    conn: Arc<futures_util::lock::Mutex<E>>,
    dialect: E::Dialect,
}

impl<E: AsyncExecutor> SingleConnection<E>
where
    E::Dialect: Clone,
{
    /// Wraps `executor`.
    pub fn new(executor: E) -> Self {
        let dialect = executor.dialect().clone();
        Self {
            conn: Arc::new(futures_util::lock::Mutex::new(executor)),
            dialect,
        }
    }
}

impl<E: AsyncExecutor> fmt::Debug for SingleConnection<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SingleConnection")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl<E> AsyncPool for SingleConnection<E>
where
    E: AsyncTxConnection + 'static,
    E::Dialect: Clone,
{
    type Conn = Pooled<futures_util::lock::OwnedMutexGuard<E>>;

    fn dialect(&self) -> &E::Dialect {
        &self.dialect
    }

    async fn get(&self) -> Result<Self::Conn, E::Error> {
        let mut conn = self.conn.clone().lock_owned().await;
        conn.clean().await?;
        Ok(Pooled(conn))
    }

    fn status(&self) -> PoolStatus {
        PoolStatus {
            max_size: 1,
            size: 1,
            idle: u32::from(self.conn.try_lock().is_some()),
        }
    }
}
