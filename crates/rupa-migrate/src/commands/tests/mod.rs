#![allow(dead_code)]

#[cfg(feature = "cli")]
mod cli;
mod create;
mod redact;
mod render;

use std::path::PathBuf;

use super::*;
use crate::{
    ArtifactInput, Catalog, Metadata, MigrationInfo, MigrationKind, Policy, Version, VersionScheme,
};

/// A catalog entry with static metadata.
struct Entry(&'static Metadata);

impl MigrationInfo for Entry {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

/// Metadata of a transactional SQL entry at `version` with source `path`.
macro_rules! sql {
    ($version:literal, $kind:ident, $path:literal) => {
        Metadata {
            version: $version,
            description: "",
            policy: Policy::TransactionalRequired,
            kind: MigrationKind::$kind,
            inputs: &[ArtifactInput::new($path, "CREATE TABLE t (id INTEGER);")],
        }
    };
}

static ONE: Metadata = sql!("1", Incremental, "0001_create_notes.sql");
static TWO: Metadata = sql!("2", Incremental, "0002_add_slugs.sql");

/// Incrementals `1` and `2`.
const NUMBERED: [Entry; 2] = [Entry(&ONE), Entry(&TWO)];

/// A numbered catalog of `entries`.
fn catalog(entries: &[Entry]) -> Catalog<'_, Entry> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

/// A numbered version.
fn version(text: &str) -> Version {
    VersionScheme::Numbered.parse(text).expect("version")
}

/// A fresh, empty directory for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// A leftover directory from an earlier run may not exist, so its
    /// removal error is ignored.
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rupa-commands-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        std::fs::create_dir_all(&dir).expect("create the scratch directory");

        Self(dir)
    }
}

impl Drop for Scratch {
    /// Cleanup is best effort: a failure must not mask the test's own.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
