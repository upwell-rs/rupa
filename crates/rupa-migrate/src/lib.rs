//! Explicit migration catalogs, history and execution, independent of drivers.

#![warn(missing_docs)]

mod catalog;
mod macros;
mod model;
mod plan;
mod runner;
mod script;
mod session;
mod state;
mod target;
mod version;

pub use catalog::{CHECKSUM_FORMAT, Catalog, CatalogEntry, Checksum, ValidationError};
pub use model::{
    ArtifactInput, Metadata, MigrationInfo, MigrationKind, Operation, Planned, PlannedMigration,
    Policy,
};
pub use plan::Plan;
pub use runner::{ApplyReport, RunError};
pub use rupa_core::schema;
pub use script::{ScriptError, ScriptRule, validate_script};
pub use session::{Context, Error, HistoryEntry, Migration, Session};
pub use state::StateError;
pub use target::{Engine, Target};
pub use version::{Version, VersionError, VersionScheme};
