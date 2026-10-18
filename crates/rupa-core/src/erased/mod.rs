//! Executors and transactions chosen at run time.
//!
//! [`Executor`], [`Transactional`] and their async twins (with the `async`
//! feature) are generic-friendly
//! but not dyn-compatible (generic methods, `impl Future` returns, a
//! lifetime-generic `Tx<'t>`). This module erases them:
//!
//! | static | erased | dyn-compatible trait |
//! |---|---|---|
//! | `E: Executor + Transactional` | [`BoxExecutor`] | [`DynExecutor`] |
//! | `E::Tx<'t>` | [`BoxTx<'t>`](BoxTx) | [`DynTransaction`] |
//! | `E: AsyncExecutor + AsyncTransactional` | [`BoxAsyncExecutor`] | [`DynAsyncExecutor`] |
//! | `E::Tx<'t>` | [`BoxAsyncTx<'t>`](BoxAsyncTx) | [`DynAsyncTransaction`] |
//!
//! The boxed types implement the static traits again, with
//! `Dialect = DynDialect` and `Error = DynError`, so repositories run on them
//! unchanged, and `begin` on a boxed executor returns a boxed transaction.
//!
//! Capabilities of an erased executor are checked at run time
//! (`dialect().supports(..)`): [`DynDialect`] implements no `Supports<_>`, so
//! statically gated APIs (`begin_with_security`, `requires = ..` DSL
//! functions) don't compile against it. Their runtime-checked variants
//! (`try_begin_with_security`, runtime DSL functions) do.

#[cfg(feature = "async")]
mod asynchronous;

use std::fmt;

use crate::dialect::{Dialect, DialectId, DynDialect};
use crate::error::DynError;
use crate::exec::{Executor, Outcome};
use crate::ir::Statement;
use crate::query::Expect;
use crate::tx::{Transaction, Transactional, TxOptions};

#[cfg(feature = "async")]
pub use asynchronous::{BoxAsyncExecutor, BoxAsyncTx, DynAsyncExecutor, DynAsyncTransaction};

/// Dyn-compatible form of a transactional [`Executor`]; implemented for every
/// one.
pub trait DynExecutor: Send {
    /// The executor's dialect.
    fn dialect_id(&self) -> DialectId;
    /// [`Executor::execute`].
    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError>;
    /// [`Transactional::begin_with`].
    fn begin_dyn(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError>;
}

impl<E: Transactional + Send> DynExecutor for E {
    fn dialect_id(&self) -> DialectId {
        self.dialect().id()
    }

    fn execute_dyn(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.execute(statement, expect).map_err(DynError::new)
    }

    fn begin_dyn(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        Ok(BoxTx::new(self.begin_with(options).map_err(DynError::new)?))
    }
}

/// Dyn-compatible form of an open [`Transaction`]; implemented for every one.
pub trait DynTransaction<'t>: DynExecutor + 't {
    /// [`Transaction::commit`].
    fn commit_dyn(self: Box<Self>) -> Result<(), DynError>;
    /// [`Transaction::rollback`].
    fn rollback_dyn(self: Box<Self>) -> Result<(), DynError>;
}

impl<'t, T: Transaction + Transactional + Send + 't> DynTransaction<'t> for T {
    fn commit_dyn(self: Box<Self>) -> Result<(), DynError> {
        (*self).commit().map_err(DynError::new)
    }

    fn rollback_dyn(self: Box<Self>) -> Result<(), DynError> {
        (*self).rollback().map_err(DynError::new)
    }
}

/// A sync, transactional executor chosen at run time.
pub struct BoxExecutor {
    inner: Box<dyn DynExecutor>,
    dialect: DynDialect,
}

impl BoxExecutor {
    /// Erases `executor`.
    pub fn new(executor: impl Transactional + Send + 'static) -> Self {
        Self::from_box(Box::new(executor))
    }

    /// Wraps an already boxed executor.
    pub fn from_box(inner: Box<dyn DynExecutor>) -> Self {
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxExecutor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxExecutor")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl Executor for BoxExecutor {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect)
    }
}

impl Transactional for BoxExecutor {
    type Tx<'t> = BoxTx<'t>;

    fn begin_with(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        self.inner.begin_dyn(options)
    }
}

/// A sync transaction (or savepoint) of a [`BoxExecutor`]. Rolled back when
/// dropped, by the wrapped transaction's own `Drop`.
pub struct BoxTx<'t> {
    inner: Box<dyn DynTransaction<'t> + 't>,
    dialect: DynDialect,
}

impl<'t> BoxTx<'t> {
    /// Erases an open transaction.
    pub fn new(tx: impl Transaction + Transactional + Send + 't) -> Self {
        let inner: Box<dyn DynTransaction<'t> + 't> = Box::new(tx);
        let dialect = DynDialect::new(inner.dialect_id());
        Self { inner, dialect }
    }
}

impl fmt::Debug for BoxTx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxTx")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

impl Executor for BoxTx<'_> {
    type Dialect = DynDialect;
    type Error = DynError;

    fn dialect(&self) -> &DynDialect {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, DynError> {
        self.inner.execute_dyn(statement, expect)
    }
}

impl Transaction for BoxTx<'_> {
    fn commit(self) -> Result<(), DynError> {
        self.inner.commit_dyn()
    }

    fn rollback(self) -> Result<(), DynError> {
        self.inner.rollback_dyn()
    }
}

impl Transactional for BoxTx<'_> {
    type Tx<'s>
        = BoxTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<BoxTx<'_>, DynError> {
        self.inner.begin_dyn(options)
    }
}
