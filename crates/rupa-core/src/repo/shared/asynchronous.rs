//! Explicitly synchronized access to a single shared async executor.

use std::future::Future;

use std::ops::{Deref, DerefMut};

use super::super::{AcquireAsync, AsyncMode, Repo};
use crate::exec::{AsyncExecutor, Awaited, Engine, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

impl<E, K> Repo<SharedAsync<E>, K, AsyncMode> {
    /// Shares one async executor, serializing queries behind an async mutex.
    pub fn shared(executor: E) -> Self {
        Self::new(SharedAsync::new(executor))
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

impl<E: AsyncExecutor> Engine for SharedAsyncConn<'_, E> {
    type Completion = Awaited;
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
