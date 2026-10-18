//! Async connection sources, and one executor shared behind an async lock.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::PoisonError;

use super::{Repo, RepoView, Shared};
use crate::dialect::{Dialect, Supports, caps};
use crate::error::TxError;
use crate::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::pool::{AsyncPool, OwnedAsyncTx};
use crate::query::Expect;
use crate::security::{SecureTx, SecurityContext, apply_security, check_native_rls};
use crate::tx::{AsyncTransaction, TxOptions};

impl<E, K> Repo<SharedAsync<E>, K> {
    /// A repository sharing one async executor behind an async mutex.
    pub fn shared_async(executor: E) -> Self {
        Self::new(SharedAsync::new(executor))
    }
}

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

/// A source that can open an owned async transaction for repository sessions.
pub trait AsyncTransactionSource: AcquireAsync {
    /// The owned transaction opened by this source.
    type Tx: AsyncTransaction<Dialect = Self::Dialect, Error = Self::Error> + Send;

    /// The source's dialect, for runtime capability checks.
    fn dialect(&self) -> &Self::Dialect;

    /// Opens an owned transaction.
    fn begin_async_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx, Self::Error>> + Send;

    /// Opens an owned transaction with default options.
    fn begin_async(&self) -> impl Future<Output = Result<Self::Tx, Self::Error>> + Send {
        self.begin_async_with(TxOptions::default())
    }

    /// Opens an owned transaction with row-level security applied.
    fn begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<Self::Tx>, Self::Error>> + Send
    where
        Self::Dialect: Supports<caps::NativeRls>,
    {
        async move { apply_security(self.begin_async_with(options).await?, context).await }
    }

    /// Opens an owned transaction with row-level security after a runtime capability check.
    fn try_begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> impl Future<Output = Result<SecureTx<Self::Tx>, Self::Error>> + Send
    where
        Self::Error: From<TxError>,
    {
        async move {
            check_native_rls(self.dialect())?;
            apply_security(self.begin_async_with(options).await?, context).await
        }
    }
}

impl<P: AsyncPool> AsyncTransactionSource for P {
    type Tx = OwnedAsyncTx<P::Conn>;

    fn dialect(&self) -> &Self::Dialect {
        AsyncPool::dialect(self)
    }

    fn begin_async_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<Self::Tx, Self::Error>> + Send {
        AsyncPool::begin_with(self, options)
    }
}

impl<'a, K, S: AsyncTransactionSource> RepoView<'a, K, S> {
    /// Opens a transaction from this repository's source for sharing with views.
    pub async fn begin_async(&self) -> Result<AsyncTxSession<S::Tx>, S::Error> {
        self.begin_async_with(TxOptions::default()).await
    }

    /// Opens a configured transaction from this repository's source.
    pub async fn begin_async_with(
        &self,
        options: TxOptions,
    ) -> Result<AsyncTxSession<S::Tx>, S::Error> {
        self.source
            .begin_async_with(options)
            .await
            .map(AsyncTxSession::new)
    }

    /// Opens a transaction with statically supported row-level security.
    pub async fn begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<AsyncSecureTxSession<S::Tx>, S::Error>
    where
        S::Dialect: Supports<caps::NativeRls>,
    {
        self.source
            .begin_async_with_security(options, context)
            .await
            .map(AsyncTxSession::new)
    }

    /// Opens a transaction with runtime checked row-level security.
    pub async fn try_begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<AsyncSecureTxSession<S::Tx>, S::Error>
    where
        S::Error: From<TxError>,
    {
        self.source
            .try_begin_async_with_security(options, context)
            .await
            .map(AsyncTxSession::new)
    }
}

impl<S: AsyncTransactionSource, K> Repo<S, K> {
    /// Opens a transaction from this repository's source for sharing with views.
    pub async fn begin_async(&self) -> Result<AsyncTxSession<S::Tx>, S::Error> {
        self.begin_async_with(TxOptions::default()).await
    }

    /// Opens a configured transaction from this repository's source.
    pub async fn begin_async_with(
        &self,
        options: TxOptions,
    ) -> Result<AsyncTxSession<S::Tx>, S::Error> {
        self.source
            .begin_async_with(options)
            .await
            .map(AsyncTxSession::new)
    }

    /// Opens a transaction with statically supported row-level security.
    pub async fn begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<AsyncSecureTxSession<S::Tx>, S::Error>
    where
        S::Dialect: Supports<caps::NativeRls>,
    {
        self.source
            .begin_async_with_security(options, context)
            .await
            .map(AsyncTxSession::new)
    }

    /// Opens a transaction with runtime checked row-level security.
    pub async fn try_begin_async_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<AsyncSecureTxSession<S::Tx>, S::Error>
    where
        S::Error: From<TxError>,
    {
        self.source
            .try_begin_async_with_security(options, context)
            .await
            .map(AsyncTxSession::new)
    }
}

/// A [`Shared`] sync executor is also an async source: each statement runs
/// synchronously (under the lock) and its future is complete on return. This
/// lets sync-only applications declare `async` repository methods without
/// an async runtime.
impl<E: Executor + Send> AcquireAsync for Shared<E>
where
    E::Dialect: Clone + Send,
{
    type Dialect = E::Dialect;
    type Error = E::Error;
    type Conn<'a>
        = SharedBlocking<'a, E>
    where
        Self: 'a;

    fn acquire(&self) -> impl Future<Output = Result<SharedBlocking<'_, E>, E::Error>> + Send {
        let dialect = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dialect()
            .clone();
        std::future::ready(Ok(SharedBlocking {
            shared: self,
            dialect,
        }))
    }
}

/// A [`Shared`] executor used asynchronously; locks per statement, because a
/// std mutex guard cannot be held across an `.await`.
pub struct SharedBlocking<'a, E: Executor> {
    shared: &'a Shared<E>,
    dialect: E::Dialect,
}

impl<E: Executor + Send> AsyncExecutor for SharedBlocking<'_, E>
where
    E::Dialect: Send,
{
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        &self.dialect
    }

    fn execute(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, E::Error>> + Send {
        let mut conn = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::future::ready(conn.execute(statement, expect))
    }
}

/// One async executor shared behind an async mutex: calls are serialized.
#[derive(Debug, Default)]
pub struct SharedAsync<E>(futures_util::lock::Mutex<E>);

impl<E> SharedAsync<E> {
    /// Shares `executor`.
    pub fn new(executor: E) -> Self {
        Self(futures_util::lock::Mutex::new(executor))
    }

    /// The executor, once no longer shared.
    pub fn into_inner(self) -> E {
        self.0.into_inner()
    }
}

/// A locked [`SharedAsync`] executor.
pub struct SharedAsyncConn<'a, E>(futures_util::lock::MutexGuard<'a, E>);

impl<E> Deref for SharedAsyncConn<'_, E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.0
    }
}

impl<E> DerefMut for SharedAsyncConn<'_, E> {
    fn deref_mut(&mut self) -> &mut E {
        &mut self.0
    }
}

impl<E: AsyncExecutor> AsyncExecutor for SharedAsyncConn<'_, E> {
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
        self.0.execute(statement, expect)
    }
}

impl<E: AsyncExecutor> AcquireAsync for SharedAsync<E> {
    type Dialect = E::Dialect;
    type Error = E::Error;
    type Conn<'a>
        = SharedAsyncConn<'a, E>
    where
        Self: 'a;

    async fn acquire(&self) -> Result<SharedAsyncConn<'_, E>, E::Error> {
        Ok(SharedAsyncConn(self.0.lock().await))
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

/// One application-owned async transaction shared by borrowed repository views.
pub struct AsyncTxSession<T: AsyncTransaction> {
    tx: futures_util::lock::Mutex<Option<T>>,
}

/// An async repository transaction session with row-level security applied.
pub type AsyncSecureTxSession<T> = AsyncTxSession<SecureTx<T>>;

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
