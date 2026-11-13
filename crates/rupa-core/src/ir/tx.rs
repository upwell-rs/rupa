//! Transaction control statements.

/// Transaction isolation level. Dialects without a level reject it when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationLevel {
    /// `READ UNCOMMITTED`.
    ReadUncommitted,
    /// `READ COMMITTED`.
    ReadCommitted,
    /// `REPEATABLE READ`.
    RepeatableRead,
    /// `SERIALIZABLE`.
    Serializable,
}

/// Options for starting a top-level transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxOptions {
    /// The isolation level (`None`: the database's default).
    pub isolation: Option<IsolationLevel>,
    /// Whether the transaction may only read.
    pub read_only: bool,
}

impl TxOptions {
    /// Sets the isolation level.
    pub fn isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation = Some(level);
        self
    }

    /// Makes the transaction read-only.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Whether these are the default options (which savepoints require).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Transaction control. Depths start at 1 for the first savepoint inside a
/// transaction; depth 0 is the transaction itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatement {
    /// Starts a transaction.
    Begin(TxOptions),
    /// Commits the transaction.
    Commit,
    /// Rolls the transaction back.
    Rollback,
    /// Creates savepoint `n`.
    Savepoint(u32),
    /// Releases (keeps the work of) savepoint `n`.
    ReleaseSavepoint(u32),
    /// Roll back to the savepoint and release it.
    RollbackToSavepoint(u32),
}
