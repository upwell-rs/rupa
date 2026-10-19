use std::path::PathBuf;

use rupa_migrate::VersionError;

/// A migration directory cannot be turned into a catalog.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A Cargo build-script variable is missing.
    #[error("{0} is not set; call `compile` from a build script")]
    Env(&'static str),
    /// The stream name cannot be used as a generated file name.
    #[error("migration stream {0:?} must be nonempty ASCII letters, digits, '-' or '_'")]
    Stream(String),
    /// Reading or writing a file failed.
    #[error("cannot access {path}: {source}")]
    Io {
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A path or migration source is not valid UTF-8.
    #[error("{0} is not valid UTF-8")]
    Utf8(PathBuf),
    /// Inputs must be regular files, so the checksum covers what is compiled.
    #[error("{0} is a symbolic link; migration inputs must be regular files")]
    Symlink(PathBuf),
    /// The entry does not follow the migration naming grammar.
    #[error(
        "{0}: expected `<version>_<snake_name>.sql`, `<version>_<snake_name>.rs` or a `<version>_<snake_name>/` directory containing `mod.rs`"
    )]
    Name(PathBuf),
    /// The migration type implements both `Migration` and `PlannedMigration`.
    #[error(
        "{0}: implement either `PlannedMigration` (a plan) or `Migration` (a script), not both"
    )]
    MixedKinds(String),
    /// The name would generate the reserved type name `Self`.
    #[error("{0}: this name cannot be used because its migration type would be `Self`")]
    ReservedName(PathBuf),
    /// A logical input path the runtime catalog would reject.
    #[error("{0:?} is not a portable input path; components must not contain ':' or '\\'")]
    InputPath(String),
    /// A lone carriage return the runtime catalog would reject.
    #[error("{0} contains a lone carriage return")]
    CarriageReturn(PathBuf),
    /// The file name's version is invalid for the configured scheme.
    #[error("{path}: {source}")]
    Version {
        /// The offending entry.
        path: PathBuf,
        /// The scheme's rejection.
        #[source]
        source: VersionError,
    },
    /// Two entries resolve to the same canonical version.
    #[error("{first} and {second} both declare migration version {version}")]
    Duplicate {
        /// The canonical version.
        version: String,
        /// The earlier entry.
        first: PathBuf,
        /// The later entry.
        second: PathBuf,
    },
    /// The directory contains no migrations.
    #[error("{0} contains no migrations")]
    Empty(PathBuf),
    /// A Rust migration does not parse.
    #[error("{path}: {message}")]
    Parse {
        /// Logical path of the source.
        path: String,
        /// The parser's message.
        message: String,
    },
    /// A Rust migration reaches outside its own immutable source closure.
    #[error("{path}: {violation}")]
    Isolation {
        /// Logical path of the source.
        path: String,
        /// What escaped the closure.
        violation: Violation,
    },
}

/// Code in a Rust migration that depends on something outside its own files.
///
/// Such code is neither hashed nor frozen with the migration, so a later
/// change to it would silently change what an already-applied migration does.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Violation {
    /// A `crate::` path names evolving application code.
    #[error(
        "`{0}` reaches application code; declare the shape this migration needs locally (e.g. a private `#[derive(rupa::Entity)]` struct)"
    )]
    CratePath(String),
    /// A `super::` path climbs above the migration's root module.
    #[error("`{0}` climbs above this migration's root module")]
    SuperPath(String),
    /// `#[path]` points a module at an arbitrary file.
    #[error("`#[path]` is not allowed; keep helper modules inside the migration directory")]
    PathAttribute,
    /// A single-file migration declares an out-of-line module.
    #[error(
        "out-of-line `mod {0};` needs a directory migration (`<version>_<name>/mod.rs`) so its file is part of the checksum"
    )]
    OutOfLineModule(String),
    /// An `include*!` path leaves the migration's files.
    #[error("`{0}` includes a file outside this migration; move it into a directory migration")]
    Include(String),
}
