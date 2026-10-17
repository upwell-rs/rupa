//! Executor traits. Building a query never does IO; an executor takes a built
//! [`Query<R>`] and produces `R`.
//!
//! Drivers implement one method, `execute`, which runs a statement and
//! returns an [`Outcome`]. The provided `run` decodes the outcome into `R`.
//!
//! [`Executor`] and [`AsyncExecutor`] are generic-friendly but not
//! dyn-compatible (`run` is generic, and the async methods return `impl Future`).
//! For executors chosen at run time, see [`crate::erased`].

use std::error::Error as StdError;
use std::fmt;
use std::future::Future;

use crate::dialect::Dialect;
use crate::error::ResultError;
use crate::ir::{Statement, TxStatement};
use crate::query::{Expect, Output, Query, QueryResult};
use crate::row::RowCursor;
use crate::tx::{AsyncTxConnection, TxConnection, rollback_to};

// Kept at their old paths.
pub use crate::erased::{BoxAsyncExecutor, BoxExecutor, BoxFuture, DynError};

/// What executing a statement produced, owned so it can cross await points.
pub enum Outcome {
    /// A row-returning statement's rows.
    Rows(Box<dyn RowCursor + Send>),
    /// A row-modifying statement's affected-row count.
    Affected(u64),
}

impl Outcome {
    /// Shapes the outcome into the query's result type `R`.
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
    /// The [`ResultError`] this error wraps, if it is one.
    fn result_error(&self) -> Option<&ResultError>;
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a synchronous executor",
    note = "async executors implement `AsyncExecutor`; use `.run(query).await` on those"
)]
/// A synchronous executor: runs statements on one connection (or
/// transaction).
pub trait Executor {
    /// The SQL dialect statements are rendered in.
    type Dialect: Dialect;
    /// The driver's error type.
    type Error: ExecError;

    /// This executor's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Runs one statement. `expect` says what result shape the caller
    /// wants, so drivers can stop fetching early.
    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error>;

    /// Runs a query and decodes its result.
    fn run<R: QueryResult>(&mut self, query: Query<R>) -> Result<R, Self::Error> {
        let outcome = self.execute(query.statement(), query.expect())?;
        Ok(outcome.decode()?)
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an async executor",
    note = "synchronous executors implement `Executor`"
)]
/// An asynchronous executor: runs statements on one connection (or
/// transaction).
pub trait AsyncExecutor: Send {
    /// The SQL dialect statements are rendered in.
    type Dialect: Dialect;
    /// The driver's error type.
    type Error: ExecError;

    /// This executor's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Runs one statement. `expect` says what result shape the caller
    /// wants, so drivers can stop fetching early.
    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    /// Runs a query and decodes its result.
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

/// A sync connection's transactions, driven from async code. Dropping an open
/// [`AsyncTx`](crate::tx::AsyncTx) on it rolls back immediately, as a sync
/// transaction would.
impl<E: TxConnection> AsyncTxConnection for Blocking<E> {
    fn control(
        &mut self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), E::Error>> + Send {
        std::future::ready(self.0.control(statement))
    }

    fn defer_rollback(&mut self, depth: u32) {
        if self.0.control(rollback_to(depth)).is_err() {
            self.0.defer_rollback(depth);
        }
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&mut self) -> impl Future<Output = Result<(), E::Error>> + Send {
        std::future::ready(self.0.clean())
    }
}
