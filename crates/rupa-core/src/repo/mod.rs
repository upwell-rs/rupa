//! Repositories: what `#[repository]` implements its traits for.
//!
//! `#[repository]` generates a distinct key and aliases for [`Repo`] and
//! [`RepoView`]. Both use an [`Acquire`] or [`AcquireAsync`] source: a pool
//! checks out a connection per query, while a [`TxSession`] or
//! [`AsyncTxSession`] shares one application-owned transaction.

#[cfg(feature = "async")]
mod asynchronous;
mod session;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::dialect::Dialect;
use crate::exec::{ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::pool::{OwnedTx, Pool};
use crate::query::Expect;
use crate::security::{SecureTx, SecurityContext};
use crate::tx::TxOptions;

#[cfg(feature = "async")]
pub use asynchronous::AsyncTxSession;
#[cfg(feature = "async")]
pub use asynchronous::{
    AcquireAsync, AsyncSecureTxSession, AsyncTransactionSource, SharedAsync, SharedAsyncConn,
    SharedBlocking,
};
pub use session::{TxSession, TxSessionConn};

/// A repository transaction session with row-level security applied.
pub type SecureTxSession<T> = TxSession<SecureTx<T>>;

/// A repository-specific, borrowed view of one connection source.
#[derive(Debug, Clone, Copy)]
pub struct RepoView<'a, K, S> {
    source: &'a S,
    key: PhantomData<fn() -> K>,
}

impl<'a, K, S> RepoView<'a, K, S> {
    /// Binds a repository to `source` without allocating or cloning it.
    pub const fn new(source: &'a S) -> Self {
        Self {
            source,
            key: PhantomData,
        }
    }

    /// The context through which this repository executes queries.
    pub const fn source(&self) -> &S {
        self.source
    }
}

impl<'a, K, S: Pool> RepoView<'a, K, S> {
    /// Opens a transaction from this repository's pool for sharing with views.
    pub fn begin(&self) -> Result<TxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error> {
        self.begin_with(TxOptions::default())
    }

    /// Opens a configured transaction from this repository's pool.
    pub fn begin_with(
        &self,
        options: TxOptions,
    ) -> Result<TxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error> {
        self.source.begin_with(options).map(TxSession::new)
    }

    /// Opens a transaction with row-level security applied.
    pub fn begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error>
    where
        <S::Conn as Executor>::Dialect: crate::dialect::Supports<crate::dialect::caps::NativeRls>,
    {
        self.source
            .begin_with_security(options, context)
            .map(TxSession::new)
    }
}

/// Owns a source for one repository identity `K`.
#[derive(Debug, Clone, Default)]
pub struct Repo<S, K> {
    source: S,
    key: PhantomData<fn() -> K>,
}

impl<S, K> Repo<S, K> {
    /// A repository owning `source`.
    pub const fn new(source: S) -> Self {
        Self {
            source,
            key: PhantomData,
        }
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

impl<S: Pool, K> Repo<S, K> {
    /// Opens a transaction from this repository's pool for sharing with views.
    pub fn begin(&self) -> Result<TxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error> {
        self.begin_with(TxOptions::default())
    }

    /// Opens a configured transaction from this repository's pool.
    pub fn begin_with(
        &self,
        options: TxOptions,
    ) -> Result<TxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error> {
        self.source.begin_with(options).map(TxSession::new)
    }

    /// Opens a transaction with row-level security applied.
    pub fn begin_with_security(
        &self,
        options: TxOptions,
        context: SecurityContext,
    ) -> Result<SecureTxSession<OwnedTx<S::Conn>>, <S::Conn as Executor>::Error>
    where
        <S::Conn as Executor>::Dialect: crate::dialect::Supports<crate::dialect::caps::NativeRls>,
    {
        self.source
            .begin_with_security(options, context)
            .map(TxSession::new)
    }
}

impl<E, K> Repo<Shared<E>, K> {
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
