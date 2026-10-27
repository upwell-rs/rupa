//! Shared access to one owned synchronous transaction.

#[cfg(feature = "async")]
mod asynchronous;

#[cfg(feature = "async")]
pub use asynchronous::AsyncTxSession;

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::Acquire;
use crate::error::TxError;
use crate::exec::{Engine, Executor, Immediate, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::Transaction;

/// One transaction explicitly shared by `&self` repository views.
///
/// The mutex serializes access to the same mutable transaction. For exclusive
/// ownership, bind an `&mut self` repository directly to the native transaction.
pub struct TxSession<T: Transaction> {
    tx: Mutex<Option<T>>,
}

impl<T: Transaction> TxSession<T> {
    /// Shares an already opened transaction with repository views.
    pub fn new(tx: T) -> Self {
        Self {
            tx: Mutex::new(Some(tx)),
        }
    }

    /// Commits the transaction and closes this context.
    pub fn commit(&self) -> Result<(), T::Error>
    where
        T::Error: From<TxError>,
    {
        self.take()?.commit()
    }

    /// Rolls the transaction back and closes this context.
    pub fn rollback(&self) -> Result<(), T::Error>
    where
        T::Error: From<TxError>,
    {
        self.take()?.rollback()
    }

    fn take(&self) -> Result<T, T::Error>
    where
        T::Error: From<TxError>,
    {
        self.tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| TxError::Closed.into())
    }
}

/// A serialized query on a [`TxSession`].
pub struct TxSessionConn<'a, T: Transaction>(MutexGuard<'a, Option<T>>);

impl<T: Transaction> Engine for TxSessionConn<'_, T> {
    type Completion = Immediate;
}

impl<T: Transaction> Executor for TxSessionConn<'_, T> {
    type Dialect = T::Dialect;
    type Error = T::Error;

    fn dialect(&self) -> &T::Dialect {
        self.0
            .as_ref()
            .expect("acquired transaction is open")
            .dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, T::Error> {
        self.0
            .as_mut()
            .expect("acquired transaction is open")
            .execute(statement, expect)
    }
}

impl<T: Transaction + Send> Acquire for TxSession<T>
where
    T::Error: From<TxError>,
{
    type Dialect = T::Dialect;
    type Error = T::Error;
    type Conn<'a>
        = TxSessionConn<'a, T>
    where
        Self: 'a;

    fn acquire(&self) -> Result<Self::Conn<'_>, Self::Error> {
        let guard = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            return Err(TxError::Closed.into());
        }

        Ok(TxSessionConn(guard))
    }
}
