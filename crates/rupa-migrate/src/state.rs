use crate::ScriptError;

/// Why a migration connection refused a user statement or script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StateError {
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
    /// A raw statement was refused by the session dialect's script grammar.
    #[error(transparent)]
    Script(ScriptError),
}
