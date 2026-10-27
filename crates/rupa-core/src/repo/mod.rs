//! Repository handles over native pools, connections, and transactions.
//!
//! `#[repository]` selects sync or async execution from the query declarations.
//! Owned repositories and borrowed views use the same handle implementation.
//! Transaction opening returns the native transaction; [`TxSession`] and
//! `AsyncTxSession` explicitly serialize access when multiple views share it.

#[cfg(feature = "async")]
mod asynchronous;
mod handle;
mod session;
mod shared;
mod transactions;

use crate::dialect::Dialect;
use crate::exec::{Engine, ExecError, Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;

#[cfg(feature = "async")]
pub use asynchronous::{AcquireAsync, AsyncTransactionSource};
#[cfg(feature = "async")]
pub use handle::{AsyncMode, ExclusiveAsyncMode};
pub use handle::{
    Context, ContextMut, ExclusiveSyncMode, Owned, Repo, RepoMut, RepoView, Repository, SyncMode,
};
#[cfg(feature = "async")]
pub use session::AsyncTxSession;
pub use session::{TxSession, TxSessionConn};
pub use shared::{Shared, SharedConn};
#[cfg(feature = "async")]
pub use shared::{SharedAsync, SharedAsyncConn};
pub use transactions::TransactionSource;

/// A repository trait object that can be built over a connection source `S`.
///
/// `#[repository]` implements it for `dyn Trait` of every dyn-compatible
/// repository, so DI integrations can provide `Arc<dyn Trait>` over their
/// pools without naming the generated repository types.
pub trait DynRepository<S> {
    /// The repository over `source`.
    fn over(source: S) -> std::sync::Arc<Self>;
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

impl<E: Engine + ?Sized> Engine for &mut E {
    type Completion = E::Completion;
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
