//! Async executors and transactions chosen at run time.

use std::fmt;
use std::future::Future;

use crate::dialect::{Dialect, DialectId, DynDialect};
use crate::error::DynError;
use crate::exec::{AsyncExecutor, BoxFuture, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{AsyncTransaction, AsyncTransactional, TxOptions};

/// Dyn-compatible form of a transactional [`AsyncExecutor`]; implemented for
/// every one.
pub trait DynAsyncExecutor: Send {
    /// The executor's dialect.
    fn dialect_id(&self) -> DialectId;
    /// [`AsyncExecutor::execute`].
    fn execute_dyn<'a>(
        &'a mut self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>>;
    /// [`AsyncTransactional::begin_with`].
    fn begin_dyn(&mut self, options: TxOptions) -> BoxFuture<'_, Result<BoxAsyncTx<'_>, DynError>>;
}

impl<E: AsyncTransactional> DynAsyncExecutor for E {
    fn dialect_id(&self) -> DialectId {
        self.dialect().id()
    }

    fn execute_dyn<'a>(
        &'a mut self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>> {
        Box::pin(async move { self.execute(statement, expect).await.map_err(DynError::new) })
    }

    fn begin_dyn(&mut self, options: TxOptions) -> BoxFuture<'_, Result<BoxAsyncTx<'_>, DynError>> {
        Box::pin(async move {
            let tx = self.begin_with(options).await.map_err(DynError::new)?;
            Ok(BoxAsyncTx::new(tx))
        })
    }
}

/// Dyn-compatible form of an open [`AsyncTransaction`]; implemented for every
/// one.
pub trait DynAsyncTransaction<'t>: DynAsyncExecutor + 't {
    /// [`AsyncTransaction::commit`].
    fn commit_dyn(self: Box<Self>) -> BoxFuture<'t, Result<(), DynError>>;
    /// [`AsyncTransaction::rollback`].
    fn rollback_dyn(self: Box<Self>) -> BoxFuture<'t, Result<(), DynError>>;
}

impl<'t, T: AsyncTransaction + AsyncTransactional + 't> DynAsyncTransaction<'t> for T {
    fn commit_dyn(self: Box<Self>) -> BoxFuture<'t, Result<(), DynError>> {
        Box::pin(async move { (*self).commit().await.map_err(DynError::new) })
    }

    fn rollback_dyn(self: Box<Self>) -> BoxFuture<'t, Result<(), DynError>> {
        Box::pin(async move { (*self).rollback().await.map_err(DynError::new) })
    }
}

/// An async, transactional executor chosen at run time.
pub struct BoxAsyncExecutor {
    inner: Box<dyn DynAsyncExecutor>,
    dialect: DynDialect,
}

impl BoxAsyncExecutor {
    /// Erases `executor`.
    pub fn new(executor: impl AsyncTransactional + 'static) -> Self {
        Self::from_box(Box::new(executor))
    }

    /// Wraps an already boxed executor.
    pub fn from_box(inner: Box<dyn DynAsyncExecutor>) -> Self {
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxAsyncExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxAsyncExecutor")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl AsyncExecutor for BoxAsyncExecutor {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    async fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect).await
    }
}

impl AsyncTransactional for BoxAsyncExecutor {
    type Tx<'t> = BoxAsyncTx<'t>;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<BoxAsyncTx<'_>, DynError>> + Send {
        self.inner.begin_dyn(options)
    }
}

/// An async transaction (or savepoint) of a [`BoxAsyncExecutor`], or an
/// owned transaction from a pool (`BoxAsyncTx<'static>`).
///
/// Dropped while open, the wrapped transaction defers its rollback to the
/// connection as usual.
pub struct BoxAsyncTx<'t> {
    inner: Box<dyn DynAsyncTransaction<'t> + 't>,
    dialect: DynDialect,
}

impl<'t> BoxAsyncTx<'t> {
    /// Erases an open transaction.
    pub fn new(tx: impl AsyncTransaction + AsyncTransactional + 't) -> Self {
        let inner: Box<dyn DynAsyncTransaction<'t> + 't> = Box::new(tx);
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxAsyncTx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxAsyncTx")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl AsyncExecutor for BoxAsyncTx<'_> {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    async fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect).await
    }
}

impl<'t> AsyncTransaction for BoxAsyncTx<'t> {
    fn commit(self) -> impl Future<Output = Result<(), DynError>> + Send {
        self.inner.commit_dyn()
    }

    fn rollback(self) -> impl Future<Output = Result<(), DynError>> + Send {
        self.inner.rollback_dyn()
    }
}

impl AsyncTransactional for BoxAsyncTx<'_> {
    type Tx<'s>
        = BoxAsyncTx<'s>
    where
        Self: 's;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<BoxAsyncTx<'_>, DynError>> + Send {
        self.inner.begin_dyn(options)
    }
}
