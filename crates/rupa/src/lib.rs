//! RUPA — Rust Unified Persistence API.
//!
//! Facade crate: re-exports the standalone API. DI integrations are opt-in
//! via cargo features and never compiled by default.

pub use rupa_core as core;
pub use rupa_dsl_std as dsl_std;
pub use rupa_macros as macros;
pub use rupa_sql as sql;

#[cfg(feature = "upwell")]
pub use rupa_upwell as upwell;
