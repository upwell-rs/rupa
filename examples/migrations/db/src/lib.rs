//! Persistence for the example application: entities, repositories and migrations.

#![warn(missing_docs)]

/// A note in its current shape.
///
/// Migrations never use this type: each declares the shape it needs at its
/// own version, so later changes here cannot alter applied history.
#[derive(Debug, rupa::Entity)]
#[entity(table = "notes")]
pub struct Note {
    /// Primary key.
    #[id]
    pub id: i64,
    /// Display title.
    pub title: String,
    /// URL-safe title, backfilled by migration 3.
    pub slug: Option<String>,
}

/// The catalog generated from `migrations/` by this crate's build script.
pub mod migrations {
    rupa::migrate::include_migrations!("primary");
}
