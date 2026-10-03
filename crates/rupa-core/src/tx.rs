//! Transactions.
//!
//! Two APIs over the same machinery:
//!
//! - **Guard:** `begin()` returns a `Tx` that is itself an executor, so code
//!   generic over [`Executor`] / [`AsyncExecutor`] runs unchanged inside or
//!   outside a transaction. Calling `begin()` on a `Tx` opens a nested
//!   transaction (a savepoint).
//! - **Closure:** `transaction(|tx| ..)` commits on `Ok` and rolls back on `Err`.
//!
//! A `Tx` that is dropped without `commit` or `rollback` is rolled back:
//!
//! - **Sync:** immediately, in `Drop`.
//! - **Async:** `Drop` cannot await. The executor is marked dirty instead, and
//!   the rollback is sent before the connection's next statement. A pool must
//!   check the executor's dirty flag (drivers expose it) before handing the
//!   connection out again.

use std::future::Future;

use crate::exec::{AsyncExecutor, Executor};
pub use crate::ir::{IsolationLevel, TxOptions};

/// A transaction that is still open.
pub trait Transaction: Executor + Sized {
    fn commit(self) -> Result<(), Self::Error>;
    fn rollback(self) -> Result<(), Self::Error>;
}

/// An executor that can open transactions. Transactions are themselves
/// transactional: `begin` on a `Tx` opens a savepoint.
pub trait Transactional: Executor {
    /// `Send` so a transaction can be driven from async code (and erased for
    /// DI) like any connection; every supported sync driver's connection is.
    type Tx<'t>: Transaction
        + Transactional
        + Send
        + Executor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 't;

    /// Starts a transaction with `options`. On a `Tx`, options must be the
    /// default (savepoints have none); anything else is an error.
    fn begin_with(&mut self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error>;

    fn begin(&mut self) -> Result<Self::Tx<'_>, Self::Error> {
        self.begin_with(TxOptions::default())
    }

    /// Runs `f` in a transaction: commits if it returns `Ok`, rolls back if it
    /// returns `Err` or panics.
    ///
    /// The closure's transaction borrows from this call (`Tx<'a>`), rather than
    /// being higher-ranked: with a GAT `where Self: 't`, a higher-ranked
    /// lifetime would force `Self: 'static` and rule out nested calls on a `Tx`.
    fn transaction<'a, T, E, F>(&'a mut self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Self::Tx<'a>) -> Result<T, E>,
        E: From<Self::Error>,
    {
        let mut tx = self.begin()?;
        match f(&mut tx) {
            Ok(value) => {
                tx.commit()?;
                Ok(value)
            }
            Err(e) => {
                // The original error wins; a failed rollback leaves the
                // connection marked for cleanup by the driver.
                let _ = tx.rollback();
                Err(e)
            }
        }
    }
}

/// An open async transaction.
pub trait AsyncTransaction: AsyncExecutor + Sized {
    fn commit(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn rollback(self) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// An async executor that can open transactions.
pub trait AsyncTransactional: AsyncExecutor {
    type Tx<'t>: AsyncTransaction
        + AsyncTransactional
        + AsyncExecutor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 't;

    fn begin_with(
        &mut self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx<'_>, Self::Error>> + Send;

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
