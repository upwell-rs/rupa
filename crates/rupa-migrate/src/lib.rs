//! Explicit migration catalogs, history and execution, independent of drivers.
//!
//! Execution comes in two modes with identical names, selected by import:
//! `sync` (feature `sync`) and `asynchronous` (feature `async`). With
//! exactly one mode enabled, its `Session`, `Migration`, `Context`, `Apply`,
//! `Inspect` and `Recover` are also re-exported here; with both enabled, import them
//! from the mode module to avoid ambiguity.

#![warn(missing_docs)]

#[cfg(feature = "async")]
pub mod asynchronous;
mod catalog;
mod error;
mod macros;
mod model;
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

pub use catalog::{CHECKSUM_FORMAT, Catalog, CatalogEntry, Checksum, ValidationError};
pub use error::{Error, HistoryEntry, Provenance};
pub use model::{
    ArtifactInput, Metadata, MigrationInfo, MigrationKind, Operation, Planned, PlannedMigration,
    Policy,
};
pub use plan::Plan;
pub use recovery::{Recovery, RecoveryAction, RecoveryError};
pub use report::{ApplyReport, RunError};
pub use rupa_core::schema;
pub use script::{ScriptError, ScriptRule, split_script, validate_script};
pub use state::StateError;
pub use status::{
    ApplyPlan, Attempt, Blocker, Classification, Exclusion, Execution, Inspection, Journal,
    Records, Status, Step,
};
pub use target::{Engine, Target};
pub use version::{Version, VersionError, VersionScheme};

/// The mode's method traits, anonymously, when exactly one mode is enabled:
/// `catalog.apply(connection)` and session calls then need no import.
pub mod prelude {
    #[cfg(all(feature = "async", not(feature = "sync")))]
    pub use crate::asynchronous::{Apply as _, Inspect as _, Recover as _, Session as _};
    #[cfg(all(feature = "sync", not(feature = "async")))]
    pub use crate::sync::{Apply as _, Inspect as _, Recover as _, Session as _};
}

#[cfg(all(feature = "async", not(feature = "sync")))]
pub use asynchronous::{Apply, Context, Inspect, Migration, Recover, Session};
#[cfg(all(feature = "sync", not(feature = "async")))]
pub use sync::{Apply, Context, Inspect, Migration, Recover, Session};
