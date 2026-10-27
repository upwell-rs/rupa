use std::process::ExitCode;

use crate::Error;

/// How a command ended, as a stable process exit status.
///
/// Scripts may rely on each code: `0` success, `1` failed, `2` usage, `3`
/// blocked, `4` busy, `5` uncertain, `6` reconciled and `7` migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Exit {
    /// The command did what it was asked; a read found nothing blocking. `0`.
    Success,
    /// The database or the filesystem failed, or the target could not be
    /// reached. `1`.
    Failed,
    /// The arguments are invalid. `2`.
    Usage,
    /// The command refused before changing anything: history, coverage or
    /// the catalog does not allow it, the target needs adoption, or an
    /// operator's assertion was refused. A status or plan that reports
    /// blockers also ends here. `3`.
    Blocked,
    /// Another runner owns the target, or SQLite's owner claim is set. `4`.
    Busy,
    /// An unresolved attempt's effects may persist: an operator must recover
    /// it before anything runs. `5`.
    Uncertain,
    /// Interrupted transactional attempts were recorded as rolled back and
    /// nothing ran: apply again. `6`.
    Reconciled,
    /// A migration body failed. Earlier commits of the run remain. `7`.
    Migration,
}

impl Exit {
    /// The process exit code.
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failed => 1,
            Self::Usage => 2,
            Self::Blocked => 3,
            Self::Busy => 4,
            Self::Uncertain => 5,
            Self::Reconciled => 6,
            Self::Migration => 7,
        }
    }

    /// The stable name reports use.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Usage => "usage",
            Self::Blocked => "blocked",
            Self::Busy => "busy",
            Self::Uncertain => "uncertain",
            Self::Reconciled => "reconciled",
            Self::Migration => "migration",
        }
    }

    /// How an infrastructure `error` ends a command.
    pub fn of<E: std::error::Error>(error: &Error<E>) -> Self {
        match error {
            Error::Database(_) => Self::Failed,
            Error::Validation(_)
            | Error::History(_)
            | Error::Blocked(_)
            | Error::Unmanaged
            | Error::Recovery(_)
            | Error::Adoption(_) => Self::Blocked,
            Error::MissingDown(_) => Self::Migration,
            Error::Busy => Self::Busy,
            Error::Uncertain(_) => Self::Uncertain,
            Error::Reconciled(_) => Self::Reconciled,
        }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}
