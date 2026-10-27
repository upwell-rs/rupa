//! Executor traits. Building a query never does IO; an executor takes a built
//! [`Query<R>`](crate::query::Query) and produces `R`.
//!
//! Drivers implement one method, `execute`, which runs a statement and
//! returns an [`Outcome`], and name their [`Engine::Completion`]. Queries run
//! with [`Query::execute`](crate::query::Query::execute), which decodes the
//! outcome into `R`:
//! `query.execute(&mut conn)?` on a sync engine,
//! `query.execute(&mut conn).await?` on an async one.
//!
//! [`Executor`] and the async `AsyncExecutor` are generic-friendly but not
//! dyn-compatible (the async methods return `impl Future`).
//! For executors chosen at run time, see [`crate::erased`].

#[cfg(feature = "async")]
mod asynchronous;
mod engine;

use std::error::Error as StdError;
use std::fmt;

use crate::dialect::Dialect;
use crate::error::ResultError;
use crate::ir::Statement;
use crate::query::{Expect, Output, QueryResult};
use crate::row::RowCursor;

#[cfg(feature = "async")]
pub use asynchronous::{AsyncExecutor, Awaited, Blocking, BoxFuture, complete_unboxed};
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
    note = "async executors implement `AsyncExecutor`; `query.execute(&mut engine).await` runs on those"
)]
/// A synchronous executor: runs statements on one connection (or
/// transaction). Queries run on it with [`Query::execute`], which returns
/// their result.
///
/// [`Query::execute`]: crate::query::Query::execute
pub trait Executor: Engine<Completion = Immediate> {
    /// The SQL dialect statements are rendered in.
    type Dialect: Dialect;
    /// The driver's error type.
    type Error: ExecError;

    /// This executor's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Runs one statement. `expect` says what result shape the caller
    /// wants, so drivers can stop fetching early.
    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error>;
}
