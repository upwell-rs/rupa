//! [`Database`]: a type-erased, cheaply cloneable async pool.

use std::fmt;
use std::sync::Arc;

use crate::dialect::{Dialect, DynDialect};
use crate::erased::{BoxAsyncExecutor, BoxAsyncTx};
use crate::error::DynError;
use crate::exec::{Blocking, BoxFuture};
use crate::pool::{AsyncPool, PoolStatus, SingleConnection};
use crate::repo::AcquireAsync;
use crate::security::{SecureTx, SecurityContext, apply_security, check_native_rls};
use crate::tx::{AsyncTx, AsyncTxConnection, TxOptions};

/// A pool behind a dyn-compatible interface; implemented for every
/// [`AsyncPool`].
trait DynPool: Send + Sync {
    fn status(&self) -> PoolStatus;
    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>>;
    fn begin_dyn(&self, options: TxOptions)
    -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>>;
}

impl<P: AsyncPool> DynPool for P {
    fn status(&self) -> PoolStatus {
        AsyncPool::status(self)
    }

    fn get_dyn(&self) -> BoxFuture<'_, Result<BoxAsyncExecutor, DynError>> {
        Box::pin(async move {
            let conn = self.get().await.map_err(DynError::new)?;
            Ok(BoxAsyncExecutor::new(conn))
        })
    }

    fn begin_dyn(
        &self,
        options: TxOptions,
    ) -> BoxFuture<'_, Result<BoxAsyncTx<'static>, DynError>> {
        Box::pin(async move {
            let conn = self.get().await.map_err(DynError::new)?;
            let tx = AsyncTx::begin(Box::new(conn), options)
                .await
                .map_err(DynError::new)?;
            Ok(BoxAsyncTx::new(tx))
        })
    }
}

/// A database chosen at run time: a type-erased async connection pool.
///
/// Cloning is cheap and shares the pool. Typical uses:
///
/// ```ignore
/// let db = rupa::connect(&DbConfig::new("sqlite://app.db")).await?;
///
/// // `&self` repositories: one pooled connection per call.
/// let users: Arc<dyn UserRepository> = Arc::new(Repo::new(db.clone()));
///
/// // A unit of work: an owned ('static) transaction on one connection.
/// let mut tx = db.begin().await?;
/// Repo::new(&mut tx).rename(id, "x").await?;
/// tx.commit().await?;
/// ```
#[derive(Clone)]
pub struct Database {
    pool: Arc<dyn DynPool>,
    dialect: DynDialect,
}

impl Database {
    /// Erases an async pool.
    pub fn new(pool: impl AsyncPool) -> Self {
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
        E: AsyncTxConnection + 'static,
        E::Dialect: Clone,
    {
        Self::new(SingleConnection::new(executor))
    }

    /// One sync connection, shared, driven through [`Blocking`].
    pub fn single_blocking<E>(executor: E) -> Self
    where
        E: crate::tx::TxConnection + 'static,
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

    /// Opens a transaction that owns its pooled connection, so it is
    /// `'static` (e.g. for a request scope). Dropped while open, it is rolled
    /// back before the connection is reused.
    pub async fn begin_with(&self, options: TxOptions) -> Result<BoxAsyncTx<'static>, DynError> {
        self.pool.begin_dyn(options).await
    }

    /// [`begin_with`](Self::begin_with) default options.
    pub async fn begin(&self) -> Result<BoxAsyncTx<'static>, DynError> {
        self.begin_with(TxOptions::default()).await
    }

    /// Opens an owned transaction with `context` applied (see
    /// [`crate::security`]). Checked at run time: fails with
    /// [`TxError::Unsupported`](crate::TxError::Unsupported), before
    /// touching the pool, unless the dialect has native row-level security.
    pub async fn try_begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTx<BoxAsyncTx<'static>>, DynError> {
        check_native_rls(&self.dialect)?;
        apply_security(self.begin_with(options).await?, context).await
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

impl AcquireAsync for Database {
    type Dialect = DynDialect;
    type Error = DynError;
    type Conn<'a> = BoxAsyncExecutor;

    fn acquire(
        &self,
    ) -> impl std::future::Future<Output = Result<BoxAsyncExecutor, DynError>> + Send {
        self.pool.get_dyn()
    }
}
