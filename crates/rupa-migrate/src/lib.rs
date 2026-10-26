//! Explicit migration catalogs, history and execution, independent of drivers.
//!
//! Execution comes in two modes with identical names, selected by import:
//! `sync` (feature `sync`) and `asynchronous` (feature `async`). With
//! exactly one mode enabled, its `Session`, `Migration`, `Context`, `Apply`,
//! `Inspect`, `Recover`, `Revert` and `Adopt` are also re-exported here; with
//! both enabled, import them from the mode module to avoid ambiguity.

#![warn(missing_docs)]

mod adoption;
#[cfg(feature = "async")]
pub mod asynchronous;
mod catalog;
mod coverage;
mod error;
mod macros;
mod model;
mod options;
mod plan;
#[cfg(feature = "protocol")]
#[doc(hidden)]
pub mod protocol;
mod recovery;
mod report;
#[cfg(any(feature = "sync", feature = "async"))]
mod runner;
mod script;
mod state;
mod status;
#[cfg(feature = "sync")]
pub mod sync;
mod target;
mod version;

pub use adoption::{AdoptReport, Adoption, AdoptionError};
pub use catalog::{CHECKSUM_FORMAT, Catalog, CatalogEntry, Checksum, Covered, ValidationError};
pub use coverage::{Coverage, CoverageMember, CoverageSource};
pub use error::{Error, HistoryEntry, Provenance};
pub use model::{
    ArtifactInput, Metadata, MigrationInfo, MigrationKind, Operation, Planned, PlannedMigration,
    Policy, Reversal,
};
pub use options::{ApplyOptions, Init, Order, RevertTarget};
pub use plan::Plan;
pub use recovery::{Recovery, RecoveryAction, RecoveryError};
pub use report::{ApplyReport, RevertReport, RunError};
pub use rupa_core::schema;
pub use script::{ScriptError, ScriptRule, split_script, validate_script};
pub use state::StateError;
pub use status::{
    ApplyPlan, Attempt, Blocker, Classification, Direction, Exclusion, Execution, Inspection,
    Journal, Records, RevertPlan, Status, Step, Undo,
};
pub use target::{Engine, Target};
pub use version::{Version, VersionError, VersionScheme};

/// The mode's method traits, anonymously, when exactly one mode is enabled:
/// `catalog.apply(connection)` and session calls then need no import.
pub mod prelude {
    #[cfg(all(feature = "async", not(feature = "sync")))]
    pub use crate::asynchronous::{
        Adopt as _, Apply as _, Inspect as _, Recover as _, Revert as _, Session as _,
    };
    #[cfg(all(feature = "sync", not(feature = "async")))]
    pub use crate::sync::{
        Adopt as _, Apply as _, Inspect as _, Recover as _, Revert as _, Session as _,
    };
}

#[cfg(all(feature = "async", not(feature = "sync")))]
pub use asynchronous::{Adopt, Apply, Context, Inspect, Migration, Recover, Revert, Session};
#[cfg(all(feature = "sync", not(feature = "async")))]
pub use sync::{Adopt, Apply, Context, Inspect, Migration, Recover, Revert, Session};
