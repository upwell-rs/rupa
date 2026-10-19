/// An immutable named file in the artifact's executable source closure.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactInput {
    /// Stable relative name, independent of the checkout's absolute root.
    pub path: &'static str,
    /// Embedded UTF-8 source, normalized before hashing.
    pub contents: &'static str,
}

impl ArtifactInput {
    /// Declares an executable source or helper input.
    pub const fn new(path: &'static str, contents: &'static str) -> Self {
        Self { path, contents }
    }
}

/// The execution guarantee required by the complete migration body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Database effects and completed history must commit together.
    TransactionalRequired,
    /// Effects can survive failure; an attempt journal and recovery are required.
    NonTransactional,
}

/// The kind of immutable executable artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationKind {
    /// Changes the preceding managed state.
    Incremental,
    /// Initializes a fresh target with explicit coverage; execution is not in this first slice.
    Snapshot,
}

/// Immutable metadata shared by SQL and compiled Rust migrations.
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    /// Canonical version interpreted using the catalog's scheme.
    pub version: &'static str,
    /// Diagnostic description, excluded from artifact identity/checksum.
    pub description: &'static str,
    /// Required execution guarantee.
    pub policy: Policy,
    /// Incremental or snapshot identity.
    pub kind: MigrationKind,
    /// Complete declared source/helper closure; must not be empty.
    pub inputs: &'static [ArtifactInput],
}

/// Metadata inspection never executes a migration body.
pub trait MigrationInfo {
    /// Static artifact metadata, including its declared source closure.
    fn info(&self) -> &'static Metadata;

    /// Exact static operations for planned execution; imperative bodies remain opaque.
    fn planned_operations(&self) -> Option<&'static [Operation]> {
        None
    }
}

/// A static plan operation whose result is intentionally discarded.
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    /// A dialect-native script on the runner-owned session.
    Sql(&'static str),
}

/// A pure, statically inspectable migration; use [`Planned`] to execute it.
pub trait PlannedMigration: MigrationInfo {
    /// Ordered forward operations.
    const UP: &'static [Operation];
}

/// Explicit interpretation of a static plan, preserving custom execution on `T`.
#[derive(Debug, Clone, Copy)]
pub struct Planned<T>(pub T);

impl<T: PlannedMigration> MigrationInfo for Planned<T> {
    fn info(&self) -> &'static Metadata {
        self.0.info()
    }

    fn planned_operations(&self) -> Option<&'static [Operation]> {
        Some(T::UP)
    }
}
