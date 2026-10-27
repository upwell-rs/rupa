//! Executor traits. Building a query never does IO; an executor takes a built
//! [`Query<R>`] and produces `R`.
//!
//! Drivers implement one method, `execute`, which runs a statement and
//! returns an [`Outcome`]. The provided `run` decodes the outcome into `R`.
//! An executor that also names its [`Engine::Completion`] runs queries through
//! [`Query::execute`] as well: `query.execute(&mut conn)?` on a sync engine,
//! `query.execute(&mut conn).await?` on an async one.
//!
//! [`Executor`] and the async `AsyncExecutor` are generic-friendly but not
//! dyn-compatible (`run` is generic, and the async methods return `impl Future`).
//! For executors chosen at run time, see [`crate::erased`].

#[cfg(feature = "async")]
mod asynchronous;
mod engine;

use std::error::Error as StdError;
use std::fmt;

use crate::dialect::Dialect;
use crate::error::ResultError;
use crate::ir::Statement;
use crate::query::{Expect, Output, Query, QueryResult};
use crate::row::RowCursor;

#[cfg(feature = "async")]
pub use asynchronous::{AsyncExecutor, Awaited, Blocking, BoxFuture};
pub use engine::{Completion, Engine, Executed, Immediate};

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

    /// Wraps this error in the public type-erased error.
    fn into_dyn_error(self) -> crate::error::DynError
    where
        Self: Sized,
    {
        crate::error::DynError::new(self)
    }
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
