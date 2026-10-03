//! Repositories: what `#[repository]` implements its traits for.
//!
//! A [`Repo<S>`] runs a repository's queries through `S`. Which `S` works
//! depends on the methods' receivers, which the trait author chooses:
//!
//! - **`&self`** (the default for shared use, e.g. `Arc<dyn Trait>` in DI):
//!   `S` is a connection *source*. It implements [`Acquire`] (sync) or
//!   [`AcquireAsync`], and hands out a connection per call: a pool (later),
//!   or one executor behind a lock ([`Shared`], [`SharedAsync`]).
//! - **`&mut self`**: `S` is the executor itself, e.g. `Repo::new(&mut tx)`
//!   inside a transaction.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::dialect::Dialect;
use crate::exec::{AsyncExecutor, ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

/// Runs a repository's queries through `S`.
#[derive(Debug, Clone, Default)]
pub struct Repo<S> {
    source: S,
}

impl<S> Repo<S> {
    pub const fn new(source: S) -> Self {
        Self { source }
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    pub fn into_inner(self) -> S {
        self.source
    }
}

impl<E> Repo<Shared<E>> {
    /// A repository sharing one sync executor behind a mutex.
    pub fn shared(executor: E) -> Self {
        Self::new(Shared::new(executor))
    }
}

impl<E> Repo<SharedAsync<E>> {
    /// A repository sharing one async executor behind an async mutex.
    pub fn shared_async(executor: E) -> Self {
        Self::new(SharedAsync::new(executor))
    }
}

/// A source of sync connections, used through `&self`.
pub trait Acquire: Send + Sync {
    type Dialect: Dialect;
    type Error: ExecError;
    type Conn<'a>: Executor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 'a;

    fn acquire(&self) -> Result<Self::Conn<'_>, Self::Error>;
}

/// A source of async connections, used through `&self`.
pub trait AcquireAsync: Send + Sync {
    type Dialect: Dialect;
    type Error: ExecError;
    type Conn<'a>: AsyncExecutor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 'a;

    fn acquire(&self) -> impl Future<Output = Result<Self::Conn<'_>, Self::Error>> + Send;
}

// ---------------------------------------------------------------------------
// One executor behind a lock
// ---------------------------------------------------------------------------

/// One sync executor shared behind a mutex: calls are serialized.
#[derive(Debug, Default)]
pub struct Shared<E>(Mutex<E>);

impl<E> Shared<E> {
    pub fn new(executor: E) -> Self {
        Self(Mutex::new(executor))
    }

    pub fn into_inner(self) -> E {
        self.0.into_inner().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A locked [`Shared`] executor.
pub struct SharedConn<'a, E>(MutexGuard<'a, E>);

impl<E> Deref for SharedConn<'_, E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.0
    }
}

impl<E> DerefMut for SharedConn<'_, E> {
    fn deref_mut(&mut self) -> &mut E {
        &mut self.0
    }
}

impl<E: Executor> Executor for SharedConn<'_, E> {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        self.0.dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, E::Error> {
        self.0.execute(statement, expect)
    }
}

impl<E: Executor + Send> Acquire for Shared<E> {
    type Dialect = E::Dialect;
    type Error = E::Error;
    type Conn<'a>
        = SharedConn<'a, E>
    where
        Self: 'a;

    fn acquire(&self) -> Result<SharedConn<'_, E>, E::Error> {
        // A panic mid-query cannot leave a transaction open: a sync `Tx`
        // rolls back when unwinding drops it. So poisoning is recoverable.
        Ok(SharedConn(
            self.0.lock().unwrap_or_else(PoisonError::into_inner),
        ))
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
    pub fn new(executor: E) -> Self {
        Self(futures_util::lock::Mutex::new(executor))
    }

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

// ---------------------------------------------------------------------------
// Executors behind `&mut`
// ---------------------------------------------------------------------------

impl<E: Executor + ?Sized> Executor for &mut E {
    type Dialect = E::Dialect;
    type Error = E::Error;

    fn dialect(&self) -> &E::Dialect {
        (**self).dialect()
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, E::Error> {
        (**self).execute(statement, expect)
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
