//! Async executors, and sync executors driven from async code.

use std::future::Future;
use std::pin::Pin;

use super::{Completion, Engine, ExecError, Executor, Outcome};
use crate::dialect::Dialect;
use crate::ir::{Statement, TxStatement};
use crate::query::{Expect, Query, QueryResult};
use crate::tx::{AsyncTxConnection, TxConnection, rollback_to};

/// A boxed, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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

/// Completes queries through a future: `execute` returns a boxed future of
/// `Result<R, E::Error>`, to `.await`. The future is boxed because an
/// associated type cannot name the executor's `async` future on stable Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {}

impl<E: AsyncExecutor, R: QueryResult + Send> Completion<E, R> for Awaited {
    type Output<'e>
        = BoxFuture<'e, Result<R, E::Error>>
    where
        E: 'e,
        R: 'e;

    fn run<'e>(engine: &'e mut E, query: Query<R>) -> Self::Output<'e>
    where
        R: 'e,
    {
        Box::pin(engine.run(query))
    }
}

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

impl<E: Executor + Send> Engine for Blocking<E> {
    type Completion = Awaited;
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
