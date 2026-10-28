//! [`Connection`]: one trait for everything a statement can run on.
//!
//! A pool, a single connection and a transaction are all connections, and so
//! is any `&`, `&mut`, `Box`, `Rc` or `Arc` of one. Code generic over
//! `C: Connection` runs a statement with [`run`](Connection::run) and opens a
//! transaction (or a savepoint, on a transaction) with
//! [`begin_tx`](Connection::begin_tx), the same way in sync and async code.
//! Repositories hold one and use nothing else.
//!
//! A transaction a connection opened is an [`OpenTransaction`] too: it
//! ends with [`end_tx`](OpenTransaction::end_tx), committed or rolled back.
//!
//! These methods return a future. A sync connection (`Mode =`
//! [`Immediate`]) returns futures that are already complete, so sync code
//! takes their value with [`now`](crate::exec::now) and no runtime;
//! an async one (`Mode = Awaited`) returns futures to `.await`. Nothing is
//! boxed either way.
//!
//! Drivers do not implement this trait. It follows from the mode's own
//! traits: [`Executor`](crate::exec::Executor) and [`Transactional`] for sync connections,
//! `AsyncExecutor` and `AsyncTransactional` for async ones. Application code
//! that knows its mode keeps using those directly: `pool.begin()?`,
//! `tx.commit().await?`.

use std::future::{Future, ready};

use crate::dialect::Dialect;
use crate::exec::{Engine, ExecError, Immediate, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{Transaction, Transactional, TxOptions};
#[cfg(feature = "async")]
use crate::{
    exec::{AsyncExecutor, Awaited},
    tx::{AsyncTransaction, AsyncTransactional},
};

/// Anything statements run on: a pool, a connection, a transaction, or a
/// reference or smart pointer to one.
///
/// `Mode` is [`Immediate`] when every future it returns is already complete,
/// and `Awaited` when they must be awaited.
pub trait Connection {
    /// [`Immediate`] for a sync connection, `Awaited` for an async one.
    type Mode;
    /// The SQL dialect statements are rendered in.
    type Dialect: Dialect;
    /// The connection's error type.
    type Error: ExecError;
    /// A transaction opened on this connection: one that owns a pooled
    /// connection on a pool, a savepoint on a transaction.
    type Tx<'a>: OpenTransaction<Mode = Self::Mode, Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 'a;

    /// Runs one statement. `expect` says what result shape the caller wants.
    fn run(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    /// Opens a transaction with `options`; on a transaction, a savepoint
    /// (whose options must be the default).
    fn begin_tx(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send;
}

/// How engines of one mode are connections. Implemented by [`Immediate`] for
/// transactional [`Executor`](crate::exec::Executor)s and by `Awaited` for
/// transactional `AsyncExecutor`s; not public API.
#[doc(hidden)]
pub trait Mode<E: ?Sized>: Sized {
    /// The engine's dialect.
    type Dialect: Dialect;
    /// The engine's error type.
    type Error: ExecError;
    /// The engine's transaction.
    type Tx<'a>: OpenTransaction<Mode = Self, Dialect = Self::Dialect, Error = Self::Error>
    where
        E: 'a;

    /// [`Connection::run`] on `engine`.
    fn run(
        engine: &E,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    /// [`Connection::begin_tx`] on `engine`.
    fn begin_tx(
        engine: &E,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send;
}

impl<E: Transactional + ?Sized> Mode<E> for Immediate {
    type Dialect = E::Dialect;
    type Error = E::Error;
    type Tx<'a>
        = E::Tx<'a>
    where
        E: 'a;

    fn run(
        engine: &E,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        ready(engine.execute(statement, expect))
    }

    fn begin_tx(
        engine: &E,
        options: TxOptions,
    ) -> impl Future<Output = Result<E::Tx<'_>, E::Error>> + Send {
        ready(engine.begin_with(options))
    }
}

#[cfg(feature = "async")]
impl<E: AsyncTransactional + ?Sized> Mode<E> for Awaited {
    type Dialect = <E as AsyncExecutor>::Dialect;
    type Error = <E as AsyncExecutor>::Error;
    type Tx<'a>
        = E::Tx<'a>
    where
        E: 'a;

    fn run(
        engine: &E,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        AsyncExecutor::execute(engine, statement, expect)
    }

    fn begin_tx(
        engine: &E,
        options: TxOptions,
    ) -> impl Future<Output = Result<E::Tx<'_>, Self::Error>> + Send {
        AsyncTransactional::begin_with(engine, options)
    }
}

impl<E> Connection for E
where
    E: Engine + ?Sized,
    E::Completion: Mode<E>,
{
    type Mode = E::Completion;
    type Dialect = <E::Completion as Mode<E>>::Dialect;
    type Error = <E::Completion as Mode<E>>::Error;
    type Tx<'a>
        = <E::Completion as Mode<E>>::Tx<'a>
    where
        Self: 'a;

    fn run(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        <E::Completion as Mode<E>>::run(self, statement, expect)
    }

    fn begin_tx(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        <E::Completion as Mode<E>>::begin_tx(self, options)
    }
}

/// How an open transaction ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxEnd {
    /// Commit the transaction (or release the savepoint).
    Commit,
    /// Roll the transaction (or savepoint) back.
    Rollback,
}

/// An open transaction, in either mode: what [`Connection::begin_tx`] returns.
///
/// Dropping one without ending it rolls it back, as its mode's transaction
/// guard does.
pub trait OpenTransaction: Connection + Sized {
    /// Commits or rolls back the transaction.
    fn end_tx(self, end: TxEnd) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// How open transactions of one mode end. Implemented by [`Immediate`] for
/// sync transactions and by `Awaited` for async ones; not public API.
#[doc(hidden)]
pub trait Ends<T>: Mode<T> {
    /// [`OpenTransaction::end_tx`] on `tx`.
    fn end_tx(tx: T, end: TxEnd) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

impl<T: Transaction + Transactional> Ends<T> for Immediate {
    fn end_tx(tx: T, end: TxEnd) -> impl Future<Output = Result<(), T::Error>> + Send {
        ready(match end {
            TxEnd::Commit => tx.commit(),
            TxEnd::Rollback => tx.rollback(),
        })
    }
}

#[cfg(feature = "async")]
impl<T: AsyncTransaction + AsyncTransactional> Ends<T> for Awaited {
    async fn end_tx(tx: T, end: TxEnd) -> Result<(), <T as AsyncExecutor>::Error> {
        match end {
            TxEnd::Commit => tx.commit().await,
            TxEnd::Rollback => tx.rollback().await,
        }
    }
}

impl<T> OpenTransaction for T
where
    T: Engine,
    T::Completion: Ends<T>,
{
    fn end_tx(self, end: TxEnd) -> impl Future<Output = Result<(), Self::Error>> + Send {
        <T::Completion as Ends<T>>::end_tx(self, end)
    }
}

/// Runs `query` on `connection` and decodes its result. What repository
/// methods do; not public API.
#[doc(hidden)]
pub async fn complete<C, R>(connection: &C, query: crate::query::Query<R>) -> Result<R, C::Error>
where
    C: Connection + ?Sized,
    R: crate::query::QueryResult,
{
    let outcome = connection.run(query.statement(), query.expect()).await?;

    Ok(outcome.decode()?)
}
