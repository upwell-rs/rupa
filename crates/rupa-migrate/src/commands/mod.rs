//! Operator commands over the engine library callers use.
//!
//! A [`Command`] names one operation: `create` and `validate` work offline;
//! `status`, `plan` and a revert preview read the target without exclusion,
//! writes or bodies; `apply`, `revert`, `adopt` and `recover` call exactly
//! the `Apply`, `Revert`, `Adopt` and `Recover` methods an application
//! would. `sync::run` or `asynchronous::run` executes one and returns an
//! [`Outcome`]: the typed report, the failure with its causes and source
//! location, and its stable [`Exit`] status. The outcome renders for people
//! or as JSON, with a [`Redactor`] masking credentials.
//!
//! No binary ships: an application's own `main`, or a command line it
//! already has, calls `run`, or with the `cli` feature embeds
//! `cli::Migrate`.

#[cfg(feature = "cli")]
pub mod cli;
mod create;
mod exit;
mod human;
mod json;
mod redact;
mod report;
mod run;

use crate::{Adoption, ApplyOptions, Engine, Recovery, RevertTarget};

pub use create::{Authoring, Create, CreateError, Created};
pub use exit::Exit;
pub use json::REPORT_FORMAT;
pub use redact::{Redactor, redact_url};
pub use report::{Failure, Format, Outcome, Report};

/// One operator command against a catalog.
#[derive(Debug, Clone)]
pub enum Command<'a> {
    /// Writes a new migration source next to the catalog's, offline.
    Create(Create<'a>),
    /// Checks the catalog offline against what an engine's sessions execute.
    Validate(Engine),
    /// Reads the target against the catalog.
    Status,
    /// Shows what an apply with these options would run, without running it.
    Plan(ApplyOptions),
    /// Applies pending migrations under these options.
    Apply(ApplyOptions),
    /// Shows what a revert to this target would undo, without undoing it.
    RevertPlan(RevertTarget),
    /// Undoes the newest installations this target selects.
    Revert(RevertTarget),
    /// Records an operator's adoption of an unmanaged target.
    Adopt(Adoption<'a>),
    /// Records an operator's assertion about an interrupted run.
    Recover(Recovery<'a>),
}

impl Command<'_> {
    /// The command's stable name, which reports carry.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::Validate(_) => "validate",
            Self::Status => "status",
            Self::Plan(_) => "plan",
            Self::Apply(_) => "apply",
            Self::RevertPlan(_) => "revert-plan",
            Self::Revert(_) => "revert",
            Self::Adopt(_) => "adopt",
            Self::Recover(_) => "recover",
        }
    }
}

#[cfg(feature = "sync")]
pub mod sync {
    //! Commands on sync sessions.

    use crate::sync::{
        Adopt as _, Apply as _, Inspect as _, Migration, Recover as _, Revert as _, Session,
    };

    super::run::commands!();
}

#[cfg(feature = "async")]
pub mod asynchronous {
    //! Commands on async sessions.

    use crate::asynchronous::{
        Adopt as _, Apply as _, Inspect as _, Migration, Recover as _, Revert as _, Session,
    };

    super::run::commands!(async);
}

#[cfg(test)]
mod tests;
