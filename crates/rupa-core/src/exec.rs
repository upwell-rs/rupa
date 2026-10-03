//! Executor traits. Building a query never does IO; an executor takes a built
//! [`Query<R>`] and produces `R`.
//!
//! Drivers implement one method, `execute`, which runs a statement and
//! returns an [`Outcome`]. The provided `run` decodes the outcome into `R`.
//!
//! [`Executor`] and [`AsyncExecutor`] are generic-friendly but not
//! dyn-compatible (`run` is generic, and the async methods return `impl Future`).
//! For executors chosen at run time, [`BoxExecutor`] / [`BoxAsyncExecutor`]
//! erase a concrete executor behind [`DynExecutor`] / [`DynAsyncExecutor`].
//! They implement the same traits again, with `Dialect = DynDialect`.

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::dialect::{Dialect, DialectId, DynDialect};
use crate::error::ResultError;
use crate::ir::Statement;
use crate::query::{Expect, Output, Query, QueryResult};
use crate::row::RowCursor;

/// What executing a statement produced, owned so it can cross await points.
pub enum Outcome {
    Rows(Box<dyn RowCursor + Send>),
    Affected(u64),
}

impl Outcome {
    pub fn decode<R: QueryResult>(self) -> Result<R, ResultError> {
        match self {
            Outcome::Rows(mut cursor) => R::from_output(Output::Rows(&mut *cursor)),
            Outcome::Affected(n) => R::from_output(Output::Affected(n)),
        }
    }
}

impl fmt::Debug for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Rows(_) => f.write_str("Outcome::Rows(..)"),
            Outcome::Affected(n) => write!(f, "Outcome::Affected({n})"),
        }
    }
}

/// Executor error types. Every driver error can carry a [`ResultError`]
/// (wrong row count, decode failure), which callers can inspect uniformly.
pub trait ExecError: StdError + Send + Sync + 'static + From<ResultError> {
    fn result_error(&self) -> Option<&ResultError>;
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a synchronous executor",
    note = "async executors implement `AsyncExecutor`; use `.run(query).await` on those"
)]
pub trait Executor {
    type Dialect: Dialect;
    type Error: ExecError;

    fn dialect(&self) -> &Self::Dialect;

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error>;

    fn run<R: QueryResult>(&mut self, query: Query<R>) -> Result<R, Self::Error> {
        let outcome = self.execute(query.statement(), query.expect())?;
        Ok(outcome.decode()?)
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an async executor",
    note = "synchronous executors implement `Executor`"
)]
pub trait AsyncExecutor: Send {
    type Dialect: Dialect;
    type Error: ExecError;

    fn dialect(&self) -> &Self::Dialect;

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    fn run<R: QueryResult + Send>(
        &mut self,
        query: Query<R>,
    ) -> impl Future<Output = Result<R, Self::Error>> + Send {
        async move {
            let outcome = self.execute(query.statement(), query.expect()).await?;
            Ok(outcome.decode()?)
        }
    }
}

// ---------------------------------------------------------------------------
// Type erasure
// ---------------------------------------------------------------------------

/// Error of an erased executor: the driver's error, boxed, plus its
/// [`ResultError`] if it was one.
#[derive(Debug)]
pub struct DynError {
    inner: Box<dyn StdError + Send + Sync>,
    result: Option<ResultError>,
}

impl DynError {
    pub fn new<E: ExecError>(e: E) -> Self {
        let result = e.result_error().cloned();
        Self {
            inner: Box::new(e),
            result,
        }
    }

    pub fn into_inner(self) -> Box<dyn StdError + Send + Sync> {
        self.inner
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

impl ExecError for DynError {
    fn result_error(&self) -> Option<&ResultError> {
        self.result.as_ref()
    }
}

/// Dyn-compatible form of [`Executor`]; implemented for every executor.
pub trait DynExecutor: Send {
    fn dialect_id(&self) -> DialectId;
    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError>;
}

impl<E: Executor + Send> DynExecutor for E {
    fn dialect_id(&self) -> DialectId {
        self.dialect().id()
    }
    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.execute(statement, expect).map_err(DynError::new)
    }
}

/// A boxed future, as returned by [`DynAsyncExecutor`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Dyn-compatible form of [`AsyncExecutor`]; implemented for every async executor.
pub trait DynAsyncExecutor: Send {
    fn dialect_id(&self) -> DialectId;
    fn execute_dyn<'a>(
        &'a mut self,
        statement: &'a Statement,
        expect: Expect,
    ) -> BoxFuture<'a, Result<Outcome, DynError>>;
}

impl<E: AsyncExecutor> DynAsyncExecutor for E {
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
}

/// A sync executor chosen at run time.
pub struct BoxExecutor {
    inner: Box<dyn DynExecutor>,
    dialect: DynDialect,
}

impl BoxExecutor {
    pub fn new(executor: impl Executor + Send + 'static) -> Self {
        Self::from_box(Box::new(executor))
    }

    pub fn from_box(inner: Box<dyn DynExecutor>) -> Self {
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
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

/// An async executor chosen at run time.
pub struct BoxAsyncExecutor {
    inner: Box<dyn DynAsyncExecutor>,
    dialect: DynDialect,
}

impl BoxAsyncExecutor {
    pub fn new(executor: impl AsyncExecutor + 'static) -> Self {
        Self::from_box(Box::new(executor))
    }

    pub fn from_box(inner: Box<dyn DynAsyncExecutor>) -> Self {
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
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

// ---------------------------------------------------------------------------
// Sync executors used from async code
// ---------------------------------------------------------------------------

/// A sync [`Executor`] used as an [`AsyncExecutor`]: each call runs the
/// statement synchronously and returns a future that is already complete.
///
/// For sync-only applications (e.g. SQLite) that still declare `async`
/// repository methods: no async runtime is needed to drive those futures.
/// It blocks the calling thread for the duration of the statement, so inside
/// an async runtime prefer an async driver.
#[derive(Debug)]
pub struct Blocking<E>(pub E);

impl<E: Executor + Send> AsyncExecutor for Blocking<E> {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        self.0.dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        std::future::ready(self.0.execute(statement, expect))
    }
}

impl<T: crate::tx::Transaction + Send> crate::tx::AsyncTransaction for Blocking<T> {
    fn commit(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.commit())
    }

    fn rollback(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.rollback())
    }
}

impl<E: crate::tx::Transactional + Send> crate::tx::AsyncTransactional for Blocking<E> {
    type Tx<'t>
        = Blocking<E::Tx<'t>>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: crate::tx::TxOptions,
    ) -> impl Future<Output = Result<Blocking<E::Tx<'_>>, E::Error>> + Send {
        std::future::ready(self.0.begin_with(options).map(Blocking))
    }
}
