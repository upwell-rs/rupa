//! Persistence for the example application: entities, repositories and migrations.

#![warn(missing_docs)]

/// A note in its current shape.
///
/// Scripted migrations never use this type: each declares the shape it needs
/// at its own version, so later changes here cannot alter applied history.
/// Planned migrations may use it, because their checksum covers the plan it
/// produces: a change that alters an applied plan is reported, not applied.
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

/// A label attached to a note; its table is created from this entity.
#[derive(Debug, rupa::Entity)]
#[entity(table = "tags")]
pub struct Tag {
    /// Primary key, assigned by the database.
    #[id(generated)]
    pub id: i64,
    /// The tagged note.
    #[column(references = Note)]
    pub note_id: i64,
    /// The label text.
    pub label: String,
}

/// The catalog generated from `migrations/` by this crate's build script:
/// the numbered `primary` stream.
pub mod migrations {
    rupa::migrate::include_migrations!("primary");
}

/// The catalog generated from `reporting/`: the `reporting` stream, versioned
/// by UTC timestamps, which a separate database holds.
pub mod reporting {
    rupa::migrate::include_migrations!("reporting");
}
