//! Async transactions: the async counterparts of the traits and the guard
//! in the parent module.

use std::future::Future;
use std::ops::DerefMut;

use super::{TxConn, TxOptions, finish_statement, open_statement};
use crate::error::TxError;
use crate::exec::{AsyncExecutor, Awaited, Engine, Outcome};
use crate::ir::{Statement, TxStatement};
use crate::query::Expect;

/// An open async transaction.
pub trait AsyncTransaction: AsyncExecutor + Sized {
    /// Commits the transaction (or releases the savepoint).
    fn commit(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
    /// Rolls the transaction (or savepoint) back.
    fn rollback(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// An async executor that can open transactions.
pub trait AsyncTransactional: AsyncExecutor {
    /// The transaction type.
    type Tx<'t>: AsyncTransaction
        + AsyncTransactional
        + AsyncExecutor<Dialect = Self::Dialect, Error = Self::Error>
        + 't
    where
        Self: 't;

    /// Starts a transaction with `options`. On a transaction, options must be
    /// the default (savepoints have none); anything else is an error.
    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send;

    /// Starts a transaction with default options.
    fn begin(&mut self) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        self.begin_with(TxOptions::default())
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err`.
    ///
    /// The returned future is `Send` whenever `f`'s future is; with a concrete
    /// executor type the compiler sees that. Generic code that must spawn the
    /// future can use the guard API (`begin` / `commit`) instead.
    fn transaction<'a, T, E, F>(&'a mut self, f: F) -> impl Future<Output = Result<T, E>>
    where
        F: AsyncFnOnce(&mut Self::Tx<'a>) -> Result<T, E>,
        E: From<Self::Error>,
    {
        async move {
            let mut tx = self.begin().await?;
            match f(&mut tx).await {
                Ok(value) => {
                    tx.commit().await?;
                    Ok(value)
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    Err(e)
                }
            }
        }
    }
}

/// Async counterpart of [`TxConnection`](crate::tx::TxConnection). Implementing it makes the connection
/// [`AsyncTransactional`], with [`AsyncTx`] as its transaction.
///
/// Dropping an [`AsyncTx`] calls [`defer_rollback`](Self::defer_rollback),
/// which cannot await: the rollback is sent before the next statement, or by
/// [`clean`](Self::clean) when a pool takes the connection back.
pub trait AsyncTxConnection: AsyncExecutor<Error: From<TxError>> {
    /// Runs a transaction-control statement now (after any deferred rollback).
    fn control(
        &mut self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Records a rollback to `depth` (`0` = the whole transaction) that must
    /// be sent before the next statement.
    fn defer_rollback(&mut self, depth: u32);

    /// Whether a deferred rollback has not been sent yet.
    fn is_dirty(&self) -> bool;

    /// Sends the deferred rollback, if any. Pools call this before reusing a
    /// connection.
    fn clean(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// An open async transaction (`depth == 0`) or savepoint (`depth > 0`) on
/// the connection `C` holds; see [`Tx`](crate::tx::Tx).
///
/// Dropped without `commit` or `rollback`, it defers the rollback to the
/// connection (see the [module docs](crate::tx)).
pub struct AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection>,
{
    conn: C,
    depth: u32,
    /// Set only once `COMMIT`/`ROLLBACK` succeeded, so a cancelled or failed
    /// finish still leaves the rollback to `Drop`.
    done: bool,
}

type AsyncErr<C> = <TxConn<C> as AsyncExecutor>::Error;

impl<C> AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    /// Opens a top-level transaction on `conn`.
    pub async fn begin(conn: C, options: TxOptions) -> Result<Self, AsyncErr<C>> {
        Self::open(conn, 0, options).await
    }

    async fn open(mut conn: C, depth: u32, options: TxOptions) -> Result<Self, AsyncErr<C>> {
        conn.control(open_statement(depth, options)?).await?;
        Ok(Self {
            conn,
            depth,
            done: false,
        })
    }

    /// `0` for a transaction, `n` for the `n`th nested savepoint.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// The connection the transaction runs on. Statements sent through the
    /// driver's own API bypass the transaction guard; use with care.
    pub fn connection(&mut self) -> &mut TxConn<C> {
        &mut self.conn
    }

    async fn finish(mut self, commit: bool) -> Result<(), AsyncErr<C>> {
        self.conn
            .control(finish_statement(commit, self.depth))
            .await?;
        self.done = true;
        Ok(())
    }
}

impl<C> Drop for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection>,
{
    fn drop(&mut self) {
        if !self.done {
            self.conn.defer_rollback(self.depth);
        }
    }
}

impl<C> Engine for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    type Completion = Awaited;
}

impl<C> AsyncExecutor for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    type Dialect = <TxConn<C> as AsyncExecutor>::Dialect;
    type Error = AsyncErr<C>;

    fn dialect(&self) -> &Self::Dialect {
        self.conn.dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send {
        self.conn.execute(statement, expect)
    }
}

impl<C> AsyncTransaction for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    fn commit(self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.finish(true)
    }

    fn rollback(self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.finish(false)
    }
}

impl<C> AsyncTransactional for AsyncTx<C>
where
    C: DerefMut<Target: AsyncTxConnection> + Send,
{
    type Tx<'s>
        = AsyncTx<&'s mut TxConn<C>>
    where
        Self: 's;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send {
        AsyncTx::open(&mut *self.conn, self.depth + 1, options)
    }
}

impl<E: AsyncTxConnection> AsyncTransactional for E {
    type Tx<'t>
        = AsyncTx<&'t mut E>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncTx<&mut E>, E::Error>> + Send {
        AsyncTx::begin(self, options)
    }
}
