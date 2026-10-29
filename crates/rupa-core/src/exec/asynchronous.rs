//! Async executors, and sync executors driven from async code.

use std::future::Future;
use std::pin::Pin;

use super::{Completion, Engine, ExecError, Executor, Outcome};
use crate::dialect::Dialect;
use crate::ir::{Statement, TxStatement};
use crate::query::{Expect, Query, QueryResult};
use crate::tx::{
    AsyncTransaction, AsyncTransactional, AsyncTxConnection, Transaction, Transactional,
    TxConnection, TxOptions, TxSlot, rollback_to,
};

/// A boxed, `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an async executor",
    note = "synchronous executors implement `Executor`"
)]
/// An asynchronous executor: runs statements on a pool, a connection or a
/// transaction, through `&self`. Queries run on it with [`Query::execute`],
/// which returns a future of their result.
///
/// `Sync`, because the futures that run on it borrow it across `.await`.
pub trait AsyncExecutor: Send + Sync + Engine<Completion = Awaited> {
    /// The SQL dialect statements are rendered in.
    type Dialect: Dialect;
    /// The driver's error type.
    type Error: ExecError;

    /// This executor's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Runs one statement. `expect` says what result shape the caller
    /// wants, so drivers can stop fetching early.
    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;
}

/// Runs `query` on `engine` without boxing, for generated repository
/// methods, whose futures stay unboxed. Not public API: use
/// [`Query::execute`].
#[doc(hidden)]
pub async fn complete_unboxed<E: AsyncExecutor + ?Sized, R: QueryResult>(
    engine: &E,
    query: Query<R>,
) -> Result<R, E::Error> {
    let outcome = engine.execute(query.statement(), query.expect()).await?;

    Ok(outcome.decode()?)
}

/// Completes queries through a future: `execute` returns a boxed future of
/// `Result<R, E::Error>`, to `.await`. The future is boxed because an
/// associated type cannot name the executor's `async` future on stable Rust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awaited {}

impl<E: AsyncExecutor + ?Sized, R: QueryResult + Send> Completion<E, R> for Awaited {
    type Output<'e>
        = BoxFuture<'e, Result<R, E::Error>>
    where
        E: 'e,
        R: 'e;

    fn complete<'e>(engine: &'e E, query: Query<R>) -> Self::Output<'e>
    where
        R: 'e,
    {
        Box::pin(complete_unboxed(engine, query))
    }
}

/// A sync [`Executor`] used as an [`AsyncExecutor`]: each call runs the
/// statement synchronously and returns a future that is already complete.
///
/// For code written against the async traits, such as an async migration
/// runner, over a sync driver. Repositories need no adapter: every
/// [`Connection`](crate::connection::Connection), sync or async, runs their
/// async methods. It blocks the calling thread for the duration of the
/// statement, so inside an async runtime prefer an async driver.
#[derive(Debug)]
pub struct Blocking<E>(pub E);

impl<E: Executor + Send + Sync> AsyncExecutor for Blocking<E> {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        self.0.dialect()
    }

    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        std::future::ready(self.0.execute(statement, expect))
    }
}

impl<E: Executor + Send + Sync> Engine for Blocking<E> {
    type Completion = Awaited;
}

/// A sync connection's transactions, driven from async code. Dropping an open
/// [`AsyncTx`](crate::tx::AsyncTx) on it rolls back immediately, as a sync
/// transaction would.
impl<E: TxConnection + Sync> AsyncTxConnection for Blocking<E> {
    fn control(&self, statement: TxStatement) -> impl Future<Output = Result<(), E::Error>> + Send {
        std::future::ready(self.0.control(statement))
    }

    fn defer_rollback(&self, depth: u32) {
        if self.0.control(rollback_to(depth)).is_err() {
            self.0.defer_rollback(depth);
        }
    }

    fn is_dirty(&self) -> bool {
        self.0.is_dirty()
    }

    fn clean(&self) -> impl Future<Output = Result<(), E::Error>> + Send {
        std::future::ready(self.0.clean())
    }

    fn slot(&self) -> &TxSlot {
        self.0.slot()
    }
}

/// A sync executor's transactions, driven from async code: each is the sync
/// transaction, so dropping one open rolls it back immediately.
impl<E: Transactional + Send + Sync> AsyncTransactional for Blocking<E> {
    type Tx<'t>
        = Blocking<E::Tx<'t>>
    where
        Self: 't;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Blocking<E::Tx<'_>>, E::Error>> + Send {
        std::future::ready(self.0.begin_with(options).map(Blocking))
    }
}

impl<T: Transaction + Transactional + Send + Sync> AsyncTransaction for Blocking<T> {
    fn commit(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.commit())
    }

    fn rollback(self) -> impl Future<Output = Result<(), T::Error>> + Send {
        std::future::ready(self.0.rollback())
    }
}
