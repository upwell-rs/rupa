mod checksum;
mod encode;

use std::fmt;

use rupa_core::dialect::{DialectId, DynDialect};
use rupa_sql::RenderError;

use checksum::checksum;

use crate::{
    Engine, Metadata, MigrationInfo, MigrationKind, Operation, Plan, Policy, Reversal, ScriptError,
    Version, VersionError, VersionScheme,
};

/// Canonical artifact encoding and digest algorithm identity.
pub const CHECKSUM_FORMAT: &str = "rupa-artifact-v1/sha256";

/// A SHA-256 digest of the versioned canonical artifact closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checksum([u8; 32]);

impl Checksum {
    /// Restores a digest from durable history.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Raw digest bytes for durable encoding.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// A catalog cannot represent a canonical immutable artifact set.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    /// Invalid explicit stream identifier.
    #[error("migration stream must be nonempty and contain no whitespace or control characters")]
    InvalidStream,
    /// The version does not follow the configured grammar.
    #[error(transparent)]
    Version(#[from] VersionError),
    /// Two artifacts of one kind claim the same version.
    #[error("duplicate migration version {0}")]
    Duplicate(Version),
    /// A snapshot with no incremental at or below its version, so nothing
    /// for it to stand for.
    #[error("snapshot {0} covers no incremental at or below its version")]
    EmptySnapshot(Version),
    /// A snapshot that declares a down body. A snapshot is never undone:
    /// the incrementals it covers never ran individually.
    #[error("snapshot {0} declares a down body; snapshots are never undone")]
    ReversibleSnapshot(Version),
    /// An invalid or duplicate logical input path.
    #[error("invalid or duplicate artifact input path {0:?}")]
    InputPath(String),
    /// A declared source contains a lone carriage return.
    #[error("artifact input {0:?} contains a lone carriage return")]
    Source(String),
    /// Imperative Rust sources/helpers must be explicitly included.
    #[error("migration {0} has no declared artifact inputs")]
    NoInputs(String),
    /// A planned migration whose plan or down plan has no operations, such
    /// as a truncated builder chain; recording it would make the completed
    /// plan a changed checksum wherever the empty one already ran.
    #[error("planned migration {0} has no operations")]
    EmptyPlan(String),
    /// A schema operation the session's dialect cannot express. Boxed to keep
    /// run errors small.
    #[error(transparent)]
    Schema(Box<RenderError>),
    /// A script refused by the session dialect's grammar before dispatch.
    #[error(transparent)]
    Script(#[from] ScriptError),
    /// The selected session cannot execute the artifact's policy.
    #[error("migration {0}: the session cannot execute this artifact's policy")]
    Unsupported(String),
}

impl From<RenderError> for ValidationError {
    fn from(error: RenderError) -> Self {
        Self::Schema(Box::new(error))
    }
}

/// Validated static metadata, checksum and numeric order for one catalog member.
#[derive(Debug)]
pub struct CatalogEntry {
    /// The canonical parsed version.
    pub version: Version,
    /// Immutable executable source checksum.
    pub checksum: Checksum,
    /// Metadata retained for inspection without running the body.
    pub metadata: &'static Metadata,
    /// The exact plan of a planned entry, as checksummed and executed;
    /// `None` denotes an opaque Rust body.
    pub plan: Option<Plan>,
    /// The authored down body, as checksummed and executed by a revert.
    pub reversal: Reversal,
    /// A snapshot's coverage manifest: every incremental at or below its
    /// version, in version order, with the checksum it stands for. Part of
    /// the snapshot's checksum. Empty for an incremental.
    pub covers: Vec<Covered>,
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) index: usize,
}

/// An incremental a snapshot stands for, as the catalog defines it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Covered {
    /// The incremental's version.
    pub version: Version,
    /// The incremental's checksum.
    pub checksum: Checksum,
}

/// A statically typed mixed SQL/Rust catalog in one explicit stream.
#[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
pub struct Catalog<'a, M> {
    pub(crate) stream: &'a str,
    pub(crate) scheme: VersionScheme,
    pub(crate) migrations: &'a [M],
    pub(crate) entries: Vec<CatalogEntry>,
}

impl<'a, M: MigrationInfo> Catalog<'a, M> {
    /// Validates the complete catalog without database IO or migration
    /// execution. Each snapshot's coverage manifest is derived here, from
    /// the incrementals at or below its version.
    pub fn new(
        stream: &'a str,
        scheme: VersionScheme,
        migrations: &'a [M],
    ) -> Result<Self, ValidationError> {
        if stream.is_empty() || stream.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ValidationError::InvalidStream);
        }

        let mut entries = Vec::with_capacity(migrations.len());
        let mut covers = Vec::new();

        for (index, migration) in migrations.iter().enumerate() {
            let metadata = migration.info();

            entries.push(CatalogEntry {
                version: scheme.parse(metadata.version)?,
                checksum: Checksum([0; 32]),
                metadata,
                plan: migration.planned(),
                reversal: migration.reversal(),
                covers: Vec::new(),
                index,
            });
        }

        entries.sort_by_key(|e| (e.version, snapshot(e)));

        for pair in entries.windows(2) {
            if pair[0].version == pair[1].version && snapshot(&pair[0]) == snapshot(&pair[1]) {
                return Err(ValidationError::Duplicate(pair[0].version));
            }
        }

        for entry in entries.iter_mut().filter(|e| !snapshot(e)) {
            entry.checksum = checksum(stream, scheme, entry)?;
            covers.push(Covered {
                version: entry.version,
                checksum: entry.checksum,
            });
        }

        for entry in entries.iter_mut().filter(|e| snapshot(e)) {
            entry.covers = covers
                .iter()
                .copied()
                .filter(|covered| covered.version <= entry.version)
                .collect();

            if entry.reversal != Reversal::Irreversible {
                return Err(ValidationError::ReversibleSnapshot(entry.version));
            }

            if entry.covers.is_empty() {
                return Err(ValidationError::EmptySnapshot(entry.version));
            }

            entry.checksum = checksum(stream, scheme, entry)?;
        }

        Ok(Self {
            stream,
            scheme,
            migrations,
            entries,
        })
    }

    /// Checks the catalog offline against what `engine`'s sessions can
    /// execute: every kind and policy is supported, every planned script, down
    /// scripts included, passes the engine's grammar and every planned schema
    /// operation renders.
    /// Apply runs the same checks before it touches a target.
    pub fn validate(&self, engine: Engine) -> Result<(), ValidationError> {
        self.check(engine.dialect(), engine.policies())
    }

    /// Rejects what a session of `dialect` honouring `policies` cannot
    /// execute: every artifact runs under a supported policy, and every
    /// planned script and schema operation must pass the dialect.
    pub(crate) fn check(
        &self,
        dialect: DialectId,
        policies: &[Policy],
    ) -> Result<(), ValidationError> {
        for entry in &self.entries {
            if !policies.contains(&entry.metadata.policy) {
                return Err(ValidationError::Unsupported(entry.metadata.version.into()));
            }
        }

        for operation in self
            .entries
            .iter()
            .flat_map(|e| e.plan.iter().chain(e.down()))
            .flat_map(Plan::operations)
        {
            render(operation, dialect)?;
        }

        Ok(())
    }

    /// Static entries in version order, a snapshot after the incremental of
    /// its version; inspection does not run user code.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &CatalogEntry> {
        self.entries.iter()
    }
}

impl<'a, M> Catalog<'a, M> {
    /// The catalog's stream.
    pub fn stream(&self) -> &'a str {
        self.stream
    }

    /// The catalog's version scheme.
    pub fn scheme(&self) -> VersionScheme {
        self.scheme
    }

    /// The incremental entries, in version order.
    pub(crate) fn incrementals(&self) -> impl DoubleEndedIterator<Item = &CatalogEntry> {
        self.entries.iter().filter(|e| !snapshot(e))
    }

    /// The snapshot entries, in version order.
    pub(crate) fn snapshots(&self) -> impl DoubleEndedIterator<Item = &CatalogEntry> {
        self.entries.iter().filter(|e| snapshot(e))
    }
}

/// Whether `entry` is a snapshot.
pub(crate) fn snapshot(entry: &CatalogEntry) -> bool {
    entry.metadata.kind == MigrationKind::Snapshot
}

impl CatalogEntry {
    /// The planned down body, if the entry has one.
    pub fn down(&self) -> Option<&Plan> {
        match &self.reversal {
            Reversal::Planned(plan) => Some(plan),
            Reversal::Irreversible | Reversal::Imperative => None,
        }
    }
}

/// An operation's script exactly as apply sends it: a planned script
/// normalized and checked by `dialect`'s grammar, or a schema operation
/// rendered for it.
pub(crate) fn render(operation: &Operation, dialect: DialectId) -> Result<String, ValidationError> {
    match operation {
        Operation::Sql(sql) => {
            let sql = normalize(sql, "planned SQL")?;

            crate::validate_script(&sql, dialect)?;

            Ok(sql)
        }
        Operation::Schema(op) => Ok(rupa_sql::render_schema(op, &DynDialect::new(dialect))?),
    }
}

pub(crate) fn normalize(source: &str, path: &str) -> Result<String, ValidationError> {
    let normalized = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err(ValidationError::Source(path.into()));
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests;
