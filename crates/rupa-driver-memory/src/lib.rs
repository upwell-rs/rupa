//! In-memory RUPA backend. It evaluates the query IR directly and never
//! renders SQL, which keeps the IR honest: anything expressible in it must
//! have a meaning without a database.
//!
//! Semantics follow Postgres where they are observable: three-valued logic,
//! `NULL` ordering (`NULLS LAST` ascending, `NULLS FIRST` descending),
//! `LIKE`, JSON `->`/`->>`, primary-key uniqueness and `NOT NULL` columns.
//!
//! Known differences from Postgres:
//! - Text compares by byte order (Postgres' `C` collation), not by locale.
//! - Type errors surface when a row is evaluated, not when the query is planned.
//! - `->>` on a JSON object or array is unsupported, because jsonb normalizes
//!   their text form.
//! - Raw SQL, and DSL functions without an `eval`, are unsupported.
//!
//! Tables must be registered (the moral equivalent of a migration) before use.

#![warn(missing_docs)]

#[cfg(feature = "async")]
mod asynchronous;
#[cfg(feature = "dynamic")]
mod driver;
mod error;
mod eval;
#[cfg(test)]
mod join_tests;
mod store;
mod table;

use std::sync::{Mutex, MutexGuard, PoisonError};

use rupa_core::column::ColumnMeta;
use rupa_core::dialect::Memory;
use rupa_core::exec::{Engine, Executor, Immediate, Outcome};
use rupa_core::ir::{Statement, TableRef, TxStatement};
use rupa_core::tx::{Transactional, Tx, TxConnection, TxOptions};
use rupa_core::{Entity, Expect, Value};

#[cfg(feature = "async")]
pub use asynchronous::{AsyncMemoryDb, AsyncMemoryTx};
#[cfg(feature = "dynamic")]
pub use driver::MemoryDriver;
pub use error::MemoryError;
use store::Store;

/// An in-memory database: a synchronous [`Executor`]. With the `async`
/// feature, `db.into_async()` makes it an async executor (`AsyncMemoryDb`).
///
/// Statements run one at a time behind an internal lock, so a database, and
/// a transaction on it, is shared by reference.
#[derive(Debug, Default)]
pub struct MemoryDb {
    store: Mutex<Store>,
    dialect: Memory,
}

impl MemoryDb {
    /// An empty database: register tables before use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates (or empties) the table for `E`.
    pub fn register<E: Entity>(&mut self) -> &mut Self {
        self.create_table(E::TABLE, E::COLUMNS.to_vec(), E::ID_COLUMNS)
    }

    /// Creates (or empties) a table from column metadata.
    pub fn create_table(
        &mut self,
        table: TableRef,
        columns: Vec<ColumnMeta>,
        key: &[&'static str],
    ) -> &mut Self {
        self.store_mut().create_table(table, columns, key);

        self
    }

    /// All rows of a table, in insertion order.
    pub fn rows(&self, table: TableRef) -> Option<Vec<Vec<Value>>> {
        self.lock().rows(table).map(<[_]>::to_vec)
    }

    /// The state, exclusively borrowed.
    fn store_mut(&mut self) -> &mut Store {
        self.store.get_mut().unwrap_or_else(PoisonError::into_inner)
    }

    /// The state, for one statement. A statement that panicked midway may
    /// have changed some rows of a write; the poisoned lock is still usable,
    /// as a database whose write failed is.
    fn lock(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clone for MemoryDb {
    fn clone(&self) -> Self {
        Self {
            store: Mutex::new(self.lock().clone()),
            dialect: self.dialect,
        }
    }
}

impl Engine for MemoryDb {
    type Completion = Immediate;
}

impl Executor for MemoryDb {
    type Dialect = Memory;
    type Error = MemoryError;

    fn dialect(&self) -> &Memory {
        &self.dialect
    }

    fn execute(&self, statement: &Statement, _expect: Expect) -> Result<Outcome, MemoryError> {
        self.lock().run_statement(statement)
    }
}

/// A transaction (or, nested, a savepoint) on a [`MemoryDb`]. Rolled back
/// when dropped without `commit`.
pub type MemoryTx<'t> = Tx<&'t MemoryDb>;

impl TxConnection for MemoryDb {
    fn control(&self, statement: TxStatement) -> Result<(), MemoryError> {
        self.lock().control_now(statement);

        Ok(())
    }

    /// Nothing can fail in memory: roll back right away.
    fn defer_rollback(&self, depth: u32) {
        self.lock().control_now(rupa_core::tx::rollback_to(depth));
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn clean(&self) -> Result<(), MemoryError> {
        Ok(())
    }
}

impl Transactional for MemoryDb {
    type Tx<'t> = MemoryTx<'t>;

    fn begin_with(&self, options: TxOptions) -> Result<MemoryTx<'_>, MemoryError> {
        Tx::begin(self, options)
    }
}
