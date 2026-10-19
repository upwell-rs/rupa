/// Why a migration connection refused user statements or scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MigrationStateError {
    /// The connection holds no migration ownership claim.
    #[error("migration connection does not own the target")]
    NotOwner,
    /// No migration attempt is currently journaled on this connection.
    #[error("no migration attempt is in progress on this connection")]
    NoAttempt,
    /// An earlier uncertain outcome poisoned the connection; it must be discarded.
    #[error("migration connection is poisoned by an uncertain outcome and must be discarded")]
    Poisoned,
    /// The artifact transaction is not open.
    #[error("no migration artifact transaction is open")]
    NoTransaction,
}
