//! Explicit migration catalogs, history and execution, independent of drivers.

#![warn(missing_docs)]

mod catalog;
mod macros;
mod model;
mod plan;
mod runner;
mod script;
mod session;
mod version;

pub use catalog::{CHECKSUM_FORMAT, Catalog, CatalogEntry, Checksum, ValidationError};
pub use model::{
    ArtifactInput, Metadata, MigrationInfo, MigrationKind, Operation, Planned, PlannedMigration,
    Policy,
};
pub use plan::Plan;
pub use runner::{ApplyReport, RunError};
pub use rupa_core::schema;
pub use session::{Context, Error, HistoryEntry, Migration, Session};
pub use version::{Version, VersionError, VersionScheme};
