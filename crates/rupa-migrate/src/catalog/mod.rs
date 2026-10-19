mod encode;

use std::fmt;

use rupa_core::dialect::{DialectId, DynDialect};
use rupa_sql::RenderError;
use sha2::{Digest, Sha256};

use crate::{
    Metadata, MigrationInfo, MigrationKind, Operation, Plan, Policy, ScriptError, Version,
    VersionError, VersionScheme,
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
    /// Two artifacts claim the same incremental identity.
    #[error("duplicate migration version {0}")]
    Duplicate(Version),
    /// An invalid or duplicate logical input path.
    #[error("invalid or duplicate artifact input path {0:?}")]
    InputPath(String),
    /// A declared source contains a lone carriage return.
    #[error("artifact input {0:?} contains a lone carriage return")]
    Source(String),
    /// Imperative Rust sources/helpers must be explicitly included.
    #[error("migration {0} has no declared artifact inputs")]
    NoInputs(String),
    /// A planned migration whose plan has no operations, such as a truncated
    /// builder chain; recording it would make the completed plan a changed
    /// checksum wherever the empty one already ran.
    #[error("planned migration {0} has no operations")]
    EmptyPlan(String),
    /// A schema operation the session's dialect cannot express. Boxed to keep
    /// run errors small.
    #[error(transparent)]
    Schema(Box<RenderError>),
    /// A script refused by the session dialect's grammar before dispatch.
    #[error(transparent)]
    Script(#[from] ScriptError),
    /// The first vertical does not execute snapshots or nontransactional bodies.
    #[error("migration {0}: this runner supports transactional incremental artifacts only")]
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
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) index: usize,
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
    /// Validates the complete catalog without database IO or migration execution.
    pub fn new(
        stream: &'a str,
        scheme: VersionScheme,
        migrations: &'a [M],
    ) -> Result<Self, ValidationError> {
        if stream.is_empty() || stream.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(ValidationError::InvalidStream);
        }

        let mut entries = Vec::with_capacity(migrations.len());
        for (index, migration) in migrations.iter().enumerate() {
            let metadata = migration.info();
            let version = scheme.parse(metadata.version)?;
            let plan = migration.planned();
            let checksum = checksum(stream, scheme, metadata, plan.as_ref())?;

            entries.push(CatalogEntry {
                version,
                checksum,
                metadata,
                plan,
                index,
            });
        }

        entries.sort_by_key(|e| e.version);
        for pair in entries.windows(2) {
            if pair[0].version == pair[1].version {
                return Err(ValidationError::Duplicate(pair[0].version));
            }
        }

        Ok(Self {
            stream,
            scheme,
            migrations,
            entries,
        })
    }

    /// Lexically checks every planned script with the session dialect's
    /// grammar, and renders every planned schema operation for it. Runs
    /// before acquisition, so a rejected plan never touches the target.
    #[cfg_attr(not(any(feature = "sync", feature = "async")), allow(dead_code))]
    pub(crate) fn validate_plans(&self, dialect: DialectId) -> Result<(), ValidationError> {
        let operations = self
            .entries
            .iter()
            .filter_map(|e| e.plan.as_ref())
            .flat_map(Plan::operations);

        for operation in operations {
            match operation {
                Operation::Sql(sql) => {
                    crate::validate_script(&normalize(sql, "planned SQL")?, dialect)?
                }
                Operation::Schema(op) => {
                    rupa_sql::render_schema(op, &DynDialect::new(dialect))?;
                }
            }
        }

        Ok(())
    }

    /// Static ordered entries; inspection does not run user code.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &CatalogEntry> {
        self.entries.iter()
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

/// The artifact checksum. A planned entry's identity is its plan plus any
/// declared inputs (an SQL file's source); a planned Rust migration declares
/// none, so source refactors that keep its plan keep its checksum. An
/// imperative body is opaque, so its declared source closure is required.
fn checksum(
    stream: &str,
    scheme: VersionScheme,
    metadata: &Metadata,
    plan: Option<&Plan>,
) -> Result<Checksum, ValidationError> {
    let mut hash = Sha256::new();
    let mut inputs: Vec<_> = metadata.inputs.iter().collect();
    let kind = match metadata.kind {
        MigrationKind::Incremental => "incremental",
        MigrationKind::Snapshot => "snapshot",
    };
    let policy = match metadata.policy {
        Policy::TransactionalRequired => "transactional-required",
        Policy::NonTransactional => "nontransactional",
    };

    if inputs.is_empty() && plan.is_none() {
        return Err(ValidationError::NoInputs(metadata.version.into()));
    }

    if plan.is_some_and(|plan| plan.operations().is_empty()) {
        return Err(ValidationError::EmptyPlan(metadata.version.into()));
    }

    for field in [
        CHECKSUM_FORMAT,
        stream,
        scheme.as_str(),
        kind,
        metadata.version,
        "sync",
        policy,
    ] {
        hash_field(&mut hash, field.as_bytes());
    }

    match plan.map(Plan::operations) {
        Some(operations) => {
            hash_field(&mut hash, b"planned");
            hash_field(&mut hash, &(operations.len() as u64).to_be_bytes());

            for operation in operations {
                match operation {
                    Operation::Sql(sql) => {
                        let sql = normalize(sql, "planned SQL")?;

                        hash_field(&mut hash, b"sql");
                        hash_field(&mut hash, sql.as_bytes());
                    }
                    Operation::Schema(op) => {
                        hash_field(&mut hash, b"schema");
                        encode::Encoder(&mut hash).schema(op);
                    }
                }
            }
        }
        None => hash_field(&mut hash, b"imperative"),
    }

    hash_field(&mut hash, b"inputs");
    hash_field(&mut hash, &(inputs.len() as u64).to_be_bytes());
    inputs.sort_by_key(|input| input.path);
    for (index, input) in inputs.iter().enumerate() {
        if input.path.is_empty()
            || input.path.contains('\\')
            || input
                .path
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == ".." || c.contains(':'))
            || (index > 0 && inputs[index - 1].path == input.path)
        {
            return Err(ValidationError::InputPath(input.path.into()));
        }

        let contents = normalize(input.contents, input.path)?;
        hash_field(&mut hash, input.path.as_bytes());
        hash_field(&mut hash, contents.as_bytes());
    }

    Ok(Checksum(hash.finalize().into()))
}

fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

#[cfg(test)]
mod tests;
