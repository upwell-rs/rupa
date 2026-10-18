//! Explicitly shared access to one async transaction.

use std::future::Future;

use super::super::AcquireAsync;
use crate::error::TxError;
use crate::exec::{AsyncExecutor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::AsyncTransaction;

/// One async transaction explicitly shared by `&self` repository views.
///
/// The async mutex serializes queries on that transaction. Exclusive repositories
/// borrow the native transaction directly and do not need this wrapper.
pub struct AsyncTxSession<T: AsyncTransaction> {
    tx: futures_util::lock::Mutex<Option<T>>,
}

impl<T: AsyncTransaction> AsyncTxSession<T> {
    /// Shares an already opened transaction with repository views.
    pub fn new(tx: T) -> Self {
        Self {
            tx: futures_util::lock::Mutex::new(Some(tx)),
        }
    }

    /// Commits the transaction and closes this context.
    pub async fn commit(&self) -> Result<(), T::Error>
    where
        T::Error: From<TxError>,
    {
        self.take().await?.commit().await
    }

    /// Rolls the transaction back and closes this context.
    pub async fn rollback(&self) -> Result<(), T::Error>
    where
        T::Error: From<TxError>,
    {
        self.take().await?.rollback().await
    }

    async fn take(&self) -> Result<T, T::Error>
    where
        T::Error: From<TxError>,
    {
        self.tx
            .lock()
            .await
            .take()
            .ok_or_else(|| TxError::Closed.into())
    }
}

/// A serialized query on an [`AsyncTxSession`].
pub struct AsyncTxSessionConn<'a, T: AsyncTransaction>(
    futures_util::lock::MutexGuard<'a, Option<T>>,
);

impl<T: AsyncTransaction> AsyncExecutor for AsyncTxSessionConn<'_, T> {
    type Dialect = T::Dialect;
    type Error = T::Error;

    fn dialect(&self) -> &T::Dialect {
        self.0
            .as_ref()
            .expect("acquired transaction is open")
            .dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, T::Error>> + Send {
        self.0
            .as_mut()
            .expect("acquired transaction is open")
            .execute(statement, expect)
    }
}

impl<T: AsyncTransaction + Send> AcquireAsync for AsyncTxSession<T>
where
    T::Error: From<TxError>,
{
    type Dialect = T::Dialect;
    type Error = T::Error;
    type Conn<'a>
        = AsyncTxSessionConn<'a, T>
    where
        Self: 'a;

    async fn acquire(&self) -> Result<Self::Conn<'_>, Self::Error> {
        let guard = self.tx.lock().await;
        if guard.is_none() {
            return Err(TxError::Closed.into());
        }

        Ok(AsyncTxSessionConn(guard))
    }
}
