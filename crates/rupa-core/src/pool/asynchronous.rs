//! Async pools, sync pools used from async code, and one connection as a
//! pool.

use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::sync::Arc;

use super::{Pool, PoolStatus, Pooled};
use crate::error::TxError;
use crate::exec::{AsyncExecutor, Blocking, Outcome};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;
use crate::tx::{AsyncTransactional, AsyncTx, AsyncTxConnection, TxOptions, TxSlot};

/// An async transaction that owns its pooled connection; what `begin` on an
/// [`AsyncPool`] returns.
pub type OwnedAsyncTx<C> = AsyncTx<Box<C>>;

/// A pool of async connections; see [`Pool`]. It is an async executor
/// itself, implemented with [`pool_executor!`](crate::pool_executor).
pub trait AsyncPool: AsyncTransactional<Error: From<TxError>> + 'static {
    /// A pooled connection; usually [`Pooled`]`<the pool's guard>`.
    type Conn: AsyncTxConnection<Dialect = Self::Dialect, Error = Self::Error>
        + AsyncTransactional
        + 'static;

    /// Checks a connection out, waiting up to the pool's acquire timeout.
    fn get(&self) -> impl Future<Output = Result<Self::Conn, Self::Error>> + Send;

    /// The pool's current occupancy.
    fn status(&self) -> PoolStatus;
}

/// Runs `statement` on a connection checked out of `pool`. Not public API.
#[doc(hidden)]
pub async fn execute_async<P: AsyncPool>(
    pool: &P,
    statement: &Statement,
    expect: Expect,
) -> Result<Outcome, P::Error> {
    pool.get().await?.execute(statement, expect).await
}

/// Opens a transaction that owns a connection checked out of `pool`. Not
/// public API.
#[doc(hidden)]
pub async fn begin_async<P: AsyncPool>(
    pool: &P,
    options: TxOptions,
) -> Result<OwnedAsyncTx<P::Conn>, P::Error> {
    AsyncTx::begin(Box::new(pool.get().await?), options).await
}

/// A sync pool used from async code; see the [module docs](crate::pool).
impl<P: Pool> AsyncPool for Blocking<P> {
    type Conn = Blocking<P::Conn>;

    fn get(&self) -> impl Future<Output = Result<Blocking<P::Conn>, P::Error>> + Send {
        std::future::ready(self.0.get().map(Blocking))
    }

    fn status(&self) -> PoolStatus {
        self.0.status()
    }
}

impl<G: Deref<Target: AsyncExecutor> + Send + Sync> AsyncExecutor for Pooled<G> {
    type Dialect = <G::Target as AsyncExecutor>::Dialect;
    type Error = <G::Target as AsyncExecutor>::Error;

    fn dialect(&self) -> &Self::Dialect {
        self.0.dialect()
    }

    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        self.0.execute(statement, expect)
    }
}

impl<G: Deref<Target: AsyncTxConnection> + Send + Sync> AsyncTxConnection for Pooled<G> {
    fn control(
        &self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.0.control(statement)
    }

    fn defer_rollback(&self, depth: u32) {
        self.0.defer_rollback(depth);
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.0.clean()
    }

    fn slot(&self) -> &TxSlot {
        self.0.slot()
    }
}

impl<G: Deref<Target: AsyncTxConnection> + Send + Sync> AsyncTransactional for Pooled<G> {
    type Tx<'t>
        = AsyncTx<&'t Self>
    where
        Self: 't;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncTx<&Self>, Self::Error>> + Send {
        AsyncTx::begin(self, options)
    }
}

/// One async connection as an [`AsyncPool`] of size one: `get` waits until
/// the previous user dropped it.
///
/// For backends where a pool makes no sense (the in-memory database), for
/// tests, or to share one connection that has no `&self` API of its own: a
/// transaction holds the connection until it ends, so nothing else runs
/// inside it.
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

crate::pool_executor!(
    async,
    [E: AsyncTxConnection + Sync + 'static]
    SingleConnection<E>,
    |pool| &pool.dialect
);

impl<E> AsyncPool for SingleConnection<E>
where
    E: AsyncTxConnection + Sync + 'static,
{
    type Conn = Pooled<futures_util::lock::OwnedMutexGuard<E>>;

    async fn get(&self) -> Result<Self::Conn, E::Error> {
        let conn = self.conn.clone().lock_owned().await;
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
