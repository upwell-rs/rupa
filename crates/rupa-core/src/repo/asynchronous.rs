//! Async connection sources, and one executor shared behind an async lock.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::PoisonError;

use super::{Repo, Shared};
use crate::dialect::Dialect;
use crate::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

impl<E> Repo<SharedAsync<E>> {
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
