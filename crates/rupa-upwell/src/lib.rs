//! `rupa-upwell`: the Upwell integration, enabled through the `rupa` facade's
//! `upwell` features.
//!
//! - `database` (with `sync` and/or `async`): [`database!`] declarations, each
//!   a configured, migration-gated pool component with repository providers,
//!   discovered like Upwell components, plus the [`Migrations`] plugin for
//!   their commands. See [`databases`].

#[cfg(all(feature = "database", any(feature = "sync", feature = "async")))]
pub mod databases;

#[cfg(all(feature = "database", any(feature = "sync", feature = "async")))]
pub use databases::{
    Database, DatabaseSettings, MigrationSettings, Migrations, Startup, exit_code,
};

/// Paths `database!` expands to. Not public API.
#[cfg(all(feature = "database", any(feature = "sync", feature = "async")))]
#[doc(hidden)]
pub mod __private {
    pub use std::sync::Arc;

    pub use rupa_core::pool::Connect;
    pub use rupa_core::repo::DynRepository;
    pub use rupa_migrate::{Catalog, ValidationError};
    pub use upwell::{
        COMPONENTS, CONFIG_BINDINGS, ComponentDescriptor, ConfigBindingDescriptor, ContributionId,
        PROVIDERS, ProviderDescriptor, ProviderOf, linkme,
    };
}
