//! Native async acquisition and transaction sources.

use std::future::Future;

use crate::dialect::Dialect;
use crate::exec::{AsyncExecutor, ExecError, Outcome};
use crate::ir::Statement;
use crate::pool::{AsyncPool, OwnedAsyncTx};
use crate::query::Expect;
use crate::tx::{AsyncTransaction, TxOptions};

/// A source of async connections, used through `&self`.
pub trait AcquireAsync: Send + Sync {
    /// The connections' dialect.
    type Dialect: Dialect;
    /// The connections' (and the source's) error type.
    type Error: ExecError;
    /// A checked-out connection; returned to the source when dropped.
    type Conn<'a>: AsyncExecutor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 'a;

    /// Checks a connection out.
    fn acquire(&self) -> impl Future<Output = Result<Self::Conn<'_>, Self::Error>> + Send;
}

/// An async source that opens owned native transactions.
///
/// Native async pools implement this directly. Erased adapters implement the
/// same contract without changing the repository's execution flow.
pub trait AsyncTransactionSource: AcquireAsync {
    /// The native transaction returned by the source.
    type Tx: AsyncTransaction<Dialect = Self::Dialect, Error = Self::Error> + Send;

    /// The source's dialect.
    fn dialect(&self) -> &Self::Dialect;

    /// Opens a transaction with the requested options.
    fn open(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx, Self::Error>> + Send;
}

impl<P: AsyncPool> AsyncTransactionSource for P {
    type Tx = OwnedAsyncTx<P::Conn>;

    fn dialect(&self) -> &Self::Dialect {
        AsyncPool::dialect(self)
    }

    async fn open(&self, options: TxOptions) -> Result<Self::Tx, Self::Error> {
        AsyncPool::begin_with(self, options).await
    }
}

impl<E: AsyncExecutor + ?Sized> AsyncExecutor for &mut E {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        (**self).dialect()
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        (**self).execute(statement, expect)
    }
}
