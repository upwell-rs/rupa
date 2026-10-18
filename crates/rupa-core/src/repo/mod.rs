//! Repositories: what `#[repository]` implements its traits for.
//!
//! A [`Repo<S>`] runs a repository's queries through `S`. Which `S` works
//! depends on the methods' receivers, which the trait author chooses:
//!
//! - **`&self`** (the default for shared use, e.g. `Arc<dyn Trait>` in DI):
//!   `S` is a connection *source*. It implements [`Acquire`] (sync) or
//!   [`AcquireAsync`], and hands out a connection per call: a pool (every
//!   [`Pool`](crate::pool::Pool) / [`AsyncPool`](crate::pool::AsyncPool), or a
//!   [`Database`](crate::registry::Database)), or one executor behind a lock
//!   ([`Shared`], [`SharedAsync`]).
//! - **`&mut self`**: `S` is the executor itself, e.g. `Repo::new(&mut tx)`
//!   inside a transaction.

#[cfg(feature = "async")]
mod asynchronous;

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::dialect::Dialect;
use crate::exec::{ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

#[cfg(feature = "async")]
pub use asynchronous::{AcquireAsync, SharedAsync, SharedAsyncConn, SharedBlocking};

/// Runs a repository's queries through `S`.
#[derive(Debug, Clone, Default)]
pub struct Repo<S> {
    source: S,
}

impl<S> Repo<S> {
    /// A repository running its queries through `source`.
    pub const fn new(source: S) -> Self {
        Self { source }
    }

    /// The connection source (or executor).
    pub fn source(&self) -> &S {
        &self.source
    }

    /// The connection source (or executor), mutably.
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// The connection source (or executor), by value.
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

/// A source of sync connections, used through `&self`.
pub trait Acquire: Send + Sync {
    /// The connections' dialect.
    type Dialect: Dialect;
    /// The connections' (and the source's) error type.
    type Error: ExecError;
    /// A checked-out connection; returned to the source when dropped.
    type Conn<'a>: Executor<Dialect = Self::Dialect, Error = Self::Error>
    where
        Self: 'a;

    /// Checks a connection out.
    fn acquire(&self) -> Result<Self::Conn<'_>, Self::Error>;
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
