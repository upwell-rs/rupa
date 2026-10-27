//! `rupa-upwell`: the Upwell integration, enabled through the `rupa` facade's
//! `upwell` features.
//!
//! - `migrations` (with `sync` and/or `async`): [`migrations`], a plugin per
//!   migration target that adds rupa's migration commands to an Upwell
//!   application's command line.

#[cfg(all(feature = "migrations", any(feature = "sync", feature = "async")))]
pub mod migrations;
