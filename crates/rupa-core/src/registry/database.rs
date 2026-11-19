//! [`Database`]: a type-erased, cheaply cloneable async pool.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use crate::dialect::{Dialect, DynDialect};
use crate::erased::{BoxAsyncExecutor, BoxAsyncTx};
use crate::error::DynError;
use crate::exec::{AsyncExecutor, Awaited, Blocking, BoxFuture, Engine, ExecError, Outcome};
use crate::ir::Statement;
use crate::pool::{AsyncPool, PoolStatus, SingleConnection};
use crate::query::Expect;
use crate::tx::{AsyncTransactional, AsyncTxConnection, TxConnection, TxOptions};

/// A pool behind a dyn-compatible interface; implemented for every
/// [`AsyncPool`].
trait DynPool: Send + Sync {
    fn status(&self) -> PoolStatus;
    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>>;
    fn execute_dyn<'a>(
        &'a self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>>;
    fn begin_dyn(&self, options: TxOptions)
    -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>>;
}

impl<P> DynPool for P
where
    P: AsyncPool,
    for<'t> P::Tx<'t>: 'static,
{
    fn status(&self) -> PoolStatus {
        AsyncPool::status(self)
    }

    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>> {
        Box::pin(async move {
            let conn = self.get().await.map_err(ExecError::into_dyn_error)?;
            Ok(BoxAsyncExecutor::new(conn))
        })
    }

    fn execute_dyn<'a>(
        &'a self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>> {
        Box::pin(async move {
            AsyncExecutor::execute(self, statement, expect)
                .await
                .map_err(ExecError::into_dyn_error)
        })
    }

    fn begin_dyn(
        &self,
        options: TxOptions,
    ) -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>> {
        Box::pin(async move {
            let tx = AsyncTransactional::begin_with(self, options)
                .await
                .map_err(ExecError::into_dyn_error)?;
            Ok(BoxAsyncTx::new(tx))
        })
    }
}

/// A database chosen at run time: a type-erased async connection pool.
///
/// Cloning is cheap and shares the pool. It is a
/// [`Connection`](crate::connection::Connection), like the pool it erases:
///
/// ```ignore
/// let db = rupa::connect(&DbConfig::new("sqlite://app.db")).await?;
///
/// // One pooled connection per call.
/// let users = UserRepositoryRepo::new(&db);
///
/// // A unit of work: an owned ('static) transaction on one connection.
/// let tx = users.open_tx(None).await?;
/// tx.rename(id, "x").await?;
/// tx.commit().await?;
/// ```
#[derive(Clone)]
pub struct Database {
    pool: Arc<dyn DynPool>,
    dialect: DynDialect,
}

impl Database {
    /// Erases an async pool. Its transactions own their connection, and are
    /// opened by its own `begin_with`, so a pool's transaction policy is kept.
    pub fn new<P>(pool: P) -> Self
    where
        P: AsyncPool,
        for<'t> P::Tx<'t>: 'static,
    {
        let dialect = DynDialect::new(pool.dialect().id());
        Self {
            pool: Arc::new(pool),
            dialect,
        }
    }

    /// One async connection, shared: callers take turns (see
    /// [`SingleConnection`]).
    pub fn single<E>(executor: E) -> Self
    where
        E: AsyncTxConnection + Sync + 'static,
        E::Dialect: Clone,
    {
        Self::new(SingleConnection::new(executor))
    }

    /// One sync connection, shared, driven through [`Blocking`].
    pub fn single_blocking<E>(executor: E) -> Self
    where
        E: TxConnection + Sync + 'static,
        E::Dialect: Clone,
    {
        Self::single(Blocking(executor))
    }

    /// The database's dialect, for runtime capability checks.
    pub fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    /// The pool's current occupancy.
    pub fn status(&self) -> PoolStatus {
        self.pool.status()
    }

    /// Checks a connection out of the pool. It returns when dropped.
    pub async fn get(&self) -> Result<BoxAsyncExecutor, DynError> {
        self.pool.get_dyn().await
    }
}

impl fmt::Debug for Database {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("dialect", &self.dialect.id())
            .field("status", &self.status())
            .finish()
    }
}

impl Engine for Database {
    type Completion = Awaited;
}

/// Each statement runs on a connection checked out for it, inside one boxed
/// future of the erased pool.
impl AsyncExecutor for Database {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    async fn execute(&self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.pool.execute_dyn(statement, expect).await
    }
}

/// A transaction owns its pooled connection, so it is `'static` (e.g. for a
/// request scope). Dropped while open, it is rolled back before the
/// connection is reused.
impl AsyncTransactional for Database {
    type Tx<'t> = BoxAsyncTx<'static>;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<BoxAsyncTx<'static>, DynError>> + Send {
        self.pool.begin_dyn(options)
    }
}
