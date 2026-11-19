//! The memory backend as an async executor.

use std::future::Future;

use rupa_core::Entity;
use rupa_core::dialect::Memory;
use rupa_core::exec::{AsyncExecutor, Awaited, Engine, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{
    AsyncTransactional, AsyncTx, AsyncTxConnection, TxConnection, TxOptions, TxSlot,
};

use crate::{MemoryDb, MemoryError};

/// The memory backend as an [`AsyncExecutor`]. A separate type so that each
/// backend type implements exactly one executor trait, and `.execute` is
/// never ambiguous with both traits in scope.
#[derive(Debug, Clone, Default)]
pub struct AsyncMemoryDb(MemoryDb);

impl AsyncMemoryDb {
    /// An empty database: register tables before use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or empties) the table for `E`.
    pub fn register<E: Entity>(&mut self) -> &mut Self {
        self.0.register::<E>();
        self
    }

    /// The sync database underneath.
    pub fn inner(&self) -> &MemoryDb {
        &self.0
    }

    /// The sync database underneath, mutably.
    pub fn inner_mut(&mut self) -> &mut MemoryDb {
        &mut self.0
    }

    /// The sync database underneath, by value.
    pub fn into_inner(self) -> MemoryDb {
        self.0
    }
}

impl From<MemoryDb> for AsyncMemoryDb {
    fn from(db: MemoryDb) -> Self {
        Self(db)
    }
}

impl MemoryDb {
    /// The same database, as an async executor.
    pub fn into_async(self) -> AsyncMemoryDb {
        AsyncMemoryDb(self)
    }
}

impl Engine for AsyncMemoryDb {
    type Completion = Awaited;
}

impl AsyncExecutor for AsyncMemoryDb {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.0.dialect
    }

    fn execute(
        &self,
        statement: &Statement,
        _expect: Expect,
    ) -> impl Future<Output = Result<Outcome, MemoryError>> + Send {
        std::future::ready(self.0.lock().run_statement(statement))
    }
}

/// A transaction (or, nested, a savepoint) on an [`AsyncMemoryDb`]. Rolled
/// back when dropped without `commit` (immediately: nothing to await).
pub type AsyncMemoryTx<'t> = AsyncTx<&'t AsyncMemoryDb>;

impl AsyncTxConnection for AsyncMemoryDb {
    fn control(
        &self,
        statement: TxStatement,
    ) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(TxConnection::control(&self.0, statement))
    }

    fn defer_rollback(&self, depth: u32) {
        TxConnection::defer_rollback(&self.0, depth);
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn clean(&self) -> impl Future<Output = Result<(), MemoryError>> + Send {
        std::future::ready(Ok(()))
    }

    fn slot(&self) -> &TxSlot {
        TxConnection::slot(&self.0)
    }
}

impl AsyncTransactional for AsyncMemoryDb {
    type Tx<'t> = AsyncMemoryTx<'t>;

    fn begin_with(
        &self,
        options: TxOptions,
    ) -> impl Future<Output = Result<AsyncMemoryTx<'_>, MemoryError>> + Send {
        AsyncTx::begin(self, options)
    }
}
