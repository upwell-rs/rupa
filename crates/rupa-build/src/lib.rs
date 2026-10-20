//! Build-script helpers that turn RUPA project layouts into generated code.
//!
//! Generation writes only into `OUT_DIR`; the matching runtime crate includes
//! the result (for migrations, `rupa::migrate::include_migrations!`).

#![warn(missing_docs)]

mod migrations;

pub use migrations::{DEFAULT_DIR, DirectiveError, Error, Migrations, Violation};
pub use rupa_migrate::VersionScheme;
