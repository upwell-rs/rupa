//! Running a query on an engine with [`Query::execute`]: the engine's
//! [`Completion`] decides whether the result comes back directly or as a
//! future.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use super::Executor;
use crate::query::{Query, QueryResult};

/// Something queries run on through [`Query::execute`].
///
/// It names its [`Completion`]: [`Immediate`] for an [`Executor`], whose
/// `execute` returns the result, or `Awaited` for an `AsyncExecutor`, whose
/// `execute` returns a future to `.await`. Both decode the engine's raw
/// [`Outcome`](super::Outcome) into the query's result type the same way.
/// Every executor is an engine; a migration body's context is one too.
pub trait Engine {
    /// How this engine completes a query.
    type Completion;
}

/// How an engine `E` completes a query yielding `R`.
pub trait Completion<E: ?Sized, R> {
    /// What [`Query::execute`] returns.
    type Output<'e>
    where
        E: 'e,
        R: 'e;

    /// Runs `query` on `engine`.
    fn complete<'e>(engine: &'e E, query: Query<R>) -> Self::Output<'e>
    where
        R: 'e;
}

/// Completes queries synchronously: `execute` returns `Result<R, E::Error>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Immediate {}

impl<E: Executor + ?Sized, R: QueryResult> Completion<E, R> for Immediate {
    type Output<'e>
        = Result<R, E::Error>
    where
        E: 'e,
        R: 'e;

    fn complete<'e>(engine: &'e E, query: Query<R>) -> Self::Output<'e>
    where
        R: 'e,
    {
        let outcome = engine.execute(query.statement(), query.expect())?;

        Ok(outcome.decode()?)
    }
}

/// What [`Query::execute`] returns on engine `E` for a query yielding `R`.
pub type Executed<'e, E, R> = <<E as Engine>::Completion as Completion<E, R>>::Output<'e>;

impl<R> Query<R> {
    /// Runs this query on `engine`: a sync engine returns the result, an
    /// async engine a future of it.
    ///
    /// ```ignore
    /// let open = select::<Task>().filter(Task::DONE.eq(false)).all().execute(&pool)?;
    /// let open = select::<Task>().filter(Task::DONE.eq(false)).all().execute(&tx).await?;
    /// ```
    pub fn execute<'e, E>(self, engine: &'e E) -> Executed<'e, E, R>
    where
        E: Engine + ?Sized,
        E::Completion: Completion<E, R>,
        R: 'e,
    {
        <E::Completion as Completion<E, R>>::complete(engine, self)
    }
}

/// The value of a future that is already complete, as every future of an
/// [`Immediate`] connection is: it is polled once, without a runtime.
///
/// # Panics
///
/// If `future` is still pending, which only a future that actually waits can
/// be. [`Connection`](crate::connection::Connection)s whose `Mode` is
/// `Immediate` never return one.
pub fn now<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("an immediate connection returned a pending future"),
    }
}
