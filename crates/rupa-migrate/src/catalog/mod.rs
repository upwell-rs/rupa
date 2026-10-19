use std::fmt;

use sha2::{Digest, Sha256};

use crate::{
    Metadata, MigrationInfo, MigrationKind, Operation, Policy, Version, VersionError, VersionScheme,
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
    /// Executable Rust sources/helpers must be explicitly included.
    #[error("migration {0} has no declared artifact inputs")]
    NoInputs(String),
    /// Unsupported source grammar, detected before script dispatch.
    #[error("unterminated SQL quote or comment at byte {0}")]
    Sql(usize),
    /// The first vertical does not execute snapshots or nontransactional bodies.
    #[error("migration {0}: this runner supports transactional incremental artifacts only")]
    Unsupported(String),
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
    /// Exact ordered SQL for planned entries; `None` denotes an opaque Rust body.
    pub operations: Option<&'static [Operation]>,
    pub(crate) index: usize,
}

/// A statically typed mixed SQL/Rust catalog in one explicit stream.
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
            let operations = migration.planned_operations();
            let checksum = checksum(stream, scheme, metadata, operations)?;

            entries.push(CatalogEntry {
                version,
                checksum,
                metadata,
                operations,
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

    /// Applies pending migrations on a dedicated native connection, then closes it.
    /// No runner/session wrapper is required; the driver owns exclusion and cleanup.
    pub fn apply<S: crate::Session>(
        &self,
        mut connection: S,
    ) -> Result<crate::ApplyReport, crate::RunError<S::Error, M::Error>>
    where
        M: crate::Migration<S>,
    {
        crate::runner::apply(&mut connection, self)
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

fn checksum(
    stream: &str,
    scheme: VersionScheme,
    metadata: &Metadata,
    operations: Option<&[Operation]>,
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

    if inputs.is_empty() {
        return Err(ValidationError::NoInputs(metadata.version.into()));
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

    match operations {
        Some(operations) => {
            hash_field(&mut hash, b"planned");
            hash_field(&mut hash, &(operations.len() as u64).to_be_bytes());
            for operation in operations {
                match operation {
                    Operation::Sql(sql) => {
                        let sql = normalize(sql, "planned SQL")?;
                        crate::script::validate(&sql)?;
                        hash_field(&mut hash, b"sql");
                        hash_field(&mut hash, sql.as_bytes());
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
