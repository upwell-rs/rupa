//! Executors and transactions chosen at run time.
//!
//! [`Executor`], [`Transactional`] and their async twins are generic-friendly
//! but not dyn-compatible (generic methods, `impl Future` returns, a
//! lifetime-generic `Tx<'t>`). This module erases them:
//!
//! | static | erased | dyn-compatible trait |
//! |---|---|---|
//! | `E: Executor + Transactional` | [`BoxExecutor`] | [`DynExecutor`] |
//! | `E::Tx<'t>` | [`BoxTx<'t>`](BoxTx) | [`DynTransaction`] |
//! | `E: AsyncExecutor + AsyncTransactional` | [`BoxAsyncExecutor`] | [`DynAsyncExecutor`] |
//! | `E::Tx<'t>` | [`BoxAsyncTx<'t>`](BoxAsyncTx) | [`DynAsyncTransaction`] |
//!
//! The boxed types implement the static traits again, with
//! `Dialect = DynDialect` and `Error = DynError`, so repositories run on them
//! unchanged, and `begin` on a boxed executor returns a boxed transaction.
//!
//! Capabilities of an erased executor are checked at run time
//! (`dialect().supports(..)`): [`DynDialect`] implements no `Supports<_>`, so
//! statically gated APIs (`begin_with_security`, `requires = ..` DSL
//! functions) don't compile against it. Their runtime-checked variants
//! (`try_begin_with_security`, runtime DSL functions) do.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::dialect::{Dialect, DialectId, DynDialect};
use crate::error::{ResultError, TxError};
use crate::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{AsyncTransaction, AsyncTransactional, Transaction, Transactional, TxOptions};

/// A boxed, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Error of an erased executor: the driver's error, boxed, plus its
/// [`ResultError`] if it was one, so `NotFound` & co. stay inspectable.
#[derive(Debug)]
pub struct DynError {
    inner: Box<dyn StdError + Send + Sync>,
    result: Option<ResultError>,
}

impl DynError {
    /// Wraps a driver error.
    pub fn new<E: ExecError>(e: E) -> Self {
        let result = e.result_error().cloned();
        Self {
            inner: Box::new(e),
            result,
        }
    }

    /// Wraps any error that is not an executor error (a pool timeout, a
    /// configuration problem).
    pub fn other(e: impl Into<Box<dyn StdError + Send + Sync>>) -> Self {
        Self {
            inner: e.into(),
            result: None,
        }
    }

    /// The wrapped error, e.g. to downcast to the driver's error type.
    pub fn into_inner(self) -> Box<dyn StdError + Send + Sync> {
        self.inner
    }

    /// The wrapped error, by reference.
    pub fn get_ref(&self) -> &(dyn StdError + Send + Sync + 'static) {
        &*self.inner
    }

    /// The wrapped error as `T`, if it is one.
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref()
    }
}

impl fmt::Display for DynError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl StdError for DynError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.inner)
    }
}

impl From<ResultError> for DynError {
    fn from(e: ResultError) -> Self {
        Self {
            inner: Box::new(e.clone()),
            result: Some(e),
        }
    }
}

impl From<TxError> for DynError {
    fn from(e: TxError) -> Self {
        Self::other(e)
    }
}

impl ExecError for DynError {
    fn result_error(&self) -> Option<&ResultError> {
        self.result.as_ref()
    }
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// Dyn-compatible form of a transactional [`Executor`]; implemented for every
/// one.
pub trait DynExecutor: Send {
    /// The executor's dialect.
    fn dialect_id(&self) -> DialectId;
    /// [`Executor::execute`].
    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError>;
    /// [`Transactional::begin_with`].
    fn begin_dyn(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError>;
}

impl<E: Transactional + Send> DynExecutor for E {
    fn dialect_id(&self) -> DialectId {
        self.dialect().id()
    }

    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.execute(statement, expect).map_err(DynError::new)
    }

    fn begin_dyn(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        Ok(BoxTx::new(self.begin_with(options).map_err(DynError::new)?))
    }
}

/// Dyn-compatible form of an open [`Transaction`]; implemented for every one.
pub trait DynTransaction<'t>: DynExecutor + 't {
    /// [`Transaction::commit`].
    fn commit_dyn(self: Box<Self>) -> Result<(), DynError>;
    /// [`Transaction::rollback`].
    fn rollback_dyn(self: Box<Self>) -> Result<(), DynError>;
}

impl<'t, T: Transaction + Transactional + Send + 't> DynTransaction<'t> for T {
    fn commit_dyn(self: Box<Self>) -> Result<(), DynError> {
        (*self).commit().map_err(DynError::new)
    }

    fn rollback_dyn(self: Box<Self>) -> Result<(), DynError> {
        (*self).rollback().map_err(DynError::new)
    }
}

/// A sync, transactional executor chosen at run time.
pub struct BoxExecutor {
    inner: Box<dyn DynExecutor>,
    dialect: DynDialect,
}

impl BoxExecutor {
    /// Erases `executor`.
    pub fn new(executor: impl Transactional + Send + 'static) -> Self {
        Self::from_box(Box::new(executor))
    }

    /// Wraps an already boxed executor.
    pub fn from_box(inner: Box<dyn DynExecutor>) -> Self {
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxExecutor")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl Executor for BoxExecutor {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect)
    }
}

impl Transactional for BoxExecutor {
    type Tx<'t> = BoxTx<'t>;

    fn begin_with(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        self.inner.begin_dyn(options)
    }
}

/// A sync transaction (or savepoint) of a [`BoxExecutor`]. Rolled back when
/// dropped, by the wrapped transaction's own `Drop`.
pub struct BoxTx<'t> {
    inner: Box<dyn DynTransaction<'t> + 't>,
    dialect: DynDialect,
}

impl<'t> BoxTx<'t> {
    /// Erases an open transaction.
    pub fn new(tx: impl Transaction + Transactional + Send + 't) -> Self {
        let inner: Box<dyn DynTransaction<'t> + 't> = Box::new(tx);
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxTx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxTx")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl Executor for BoxTx<'_> {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect)
    }
}

impl Transaction for BoxTx<'_> {
    fn commit(self) -> Result<(), DynError> {
        self.inner.commit_dyn()
    }

    fn rollback(self) -> Result<(), DynError> {
        self.inner.rollback_dyn()
    }
}

impl Transactional for BoxTx<'_> {
    type Tx<'s>
        = BoxTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        self.inner.begin_dyn(options)
    }
}

// ---------------------------------------------------------------------------
// Async
// ---------------------------------------------------------------------------

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
