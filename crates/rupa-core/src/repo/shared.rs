//! Explicitly synchronized access to a single shared executor.

#[cfg(feature = "async")]
mod asynchronous;

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::{Acquire, Repo, SyncMode};
use crate::exec::{Engine, Executor, Immediate, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

#[cfg(feature = "async")]
pub use asynchronous::{SharedAsync, SharedAsyncConn};

impl<E, K> Repo<Shared<E>, K, SyncMode> {
    /// Shares one sync executor, serializing queries behind a mutex.
    pub fn shared(executor: E) -> Self {
        Self::new(Shared::new(executor))
    }
}

/// One sync executor shared behind a mutex: calls are serialized.
#[derive(Debug, Default)]
pub struct Shared<E>(Mutex<E>);

impl<E> Shared<E> {
    /// Shares `executor`.
    pub fn new(executor: E) -> Self {
        Self(Mutex::new(executor))
    }

    /// The executor, once no longer shared.
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

impl<E: Executor> Engine for SharedConn<'_, E> {
    type Completion = Immediate;
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
