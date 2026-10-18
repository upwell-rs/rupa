//! The backend's error type.

use rupa_core::exec::ExecError;
use rupa_core::ir::{SourceId, TableRef};
use rupa_core::{DslError, ResultError, TxError};

/// Everything that can go wrong evaluating a statement in memory.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MemoryError {
    /// The table was never registered.
    #[error("no such table {0:?}")]
    NoSuchTable(TableRef),
    /// The table has no such column.
    #[error("no such column `{0}`")]
    NoSuchColumn(&'static str),
    /// A column names a row source the statement doesn't have.
    #[error("no row source {}", .0.0)]
    NoSuchSource(SourceId),
    /// An insert or update would duplicate a primary key.
    #[error("duplicate primary key in {0:?}")]
    DuplicateKey(TableRef),
    /// `NULL` in a `NOT NULL` column.
    #[error("NULL in NOT NULL column `{0}`")]
    NotNull(&'static str),
    /// A value has the wrong type for an operation.
    #[error("type error: {0}")]
    Type(String),
    /// The statement uses something the backend can't evaluate (raw SQL,
    /// a DSL function without `eval`, ..).
    #[error("unsupported by the memory backend: {0}")]
    Unsupported(String),
    /// A DSL function failed.
    #[error(transparent)]
    Dsl(#[from] DslError),
    /// The rows did not fit the query's result type.
    #[error(transparent)]
    Result(#[from] ResultError),
    /// A write inside a read-only transaction.
    #[error("cannot write in a read-only transaction")]
    ReadOnly,
    /// The transaction API was misused.
    #[error(transparent)]
    Tx(#[from] TxError),
}

impl ExecError for MemoryError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            MemoryError::Result(e) => Some(e),
            _ => None,
        }
    }
}

impl From<MemoryError> for rupa_core::DynError {
    fn from(e: MemoryError) -> Self {
        rupa_core::DynError::new(e)
    }
}
