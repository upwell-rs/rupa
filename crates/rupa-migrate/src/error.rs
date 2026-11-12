use crate::{AdoptionError, Blocker, Checksum, RecoveryError, ValidationError, Version};

/// Migration infrastructure failed without erasing the driver's error type.
#[derive(Debug, thiserror::Error)]
pub enum Error<E: std::error::Error + 'static> {
    /// A driver operation failed.
    #[error("migration database operation failed: {0}")]
    Database(#[source] E),
    /// An artifact cannot be safely interpreted.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// Ownership, schema or history no longer matches the runner's assumptions.
    #[error("invalid migration history: {0}")]
    History(String),
    /// Installed history or coverage no longer matches the catalog, pending
    /// artifacts are late without permission or uncovered below a coverage
    /// boundary, or a revert cannot undo what it selects. Each blocker
    /// carries its artifact identity: `Missing`, `Changed`, `Duplicate`,
    /// `Late` or `Uncovered`, and for a revert `Irreversible`,
    /// `Interleaved`, `Exceeds` or `Boundary`. Nothing ran.
    #[error("migration history does not match the catalog: {0:?}")]
    Blocked(Vec<Blocker>),
    /// An entry declares an imperative down body through
    /// [`MigrationInfo::reversal`](crate::MigrationInfo::reversal), but its
    /// `Migration` impl does not provide `down`. Its attempt was rolled back.
    #[error("migration {0} declares a down body that its Migration impl does not provide")]
    MissingDown(Version),
    /// A cooperative runner already owns the target, possibly after interruption.
    #[error(
        "migration target has an existing owner; stop the old runner and reconcile before recovery"
    )]
    Busy,
    /// A nonempty database must not be initialized implicitly.
    #[error("unmanaged target contains user objects; explicit adoption is required")]
    Unmanaged,
    /// An interrupted attempt prevents blind replay.
    #[error("migration attempt {0} is unresolved; explicit reconciliation is required")]
    Uncertain(i64),
    /// The run found interrupted transactional attempts, which provably
    /// never committed, recorded them as rolled back, and stopped without
    /// running anything. A new apply runs their artifacts again.
    #[error("rolled back interrupted migration attempts {0:?}; apply again to run them")]
    Reconciled(Vec<i64>),
    /// An operator's recovery assertion was refused before anything was recorded.
    #[error(transparent)]
    Recovery(#[from] RecoveryError),
    /// An operator's adoption was refused before anything was recorded.
    #[error(transparent)]
    Adoption(#[from] AdoptionError),
}

/// Durable successfully executed incremental history. Covered incrementals
/// are not installations: [`Coverage`](crate::Coverage) records them.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Strictly increasing installation order, separate from version order.
    pub installation_rank: u64,
    /// Canonical logical version.
    pub version: String,
    /// Checksum of the installed source closure.
    pub checksum: Checksum,
    /// Whether the runner observed completion or an operator asserted it.
    pub provenance: Provenance,
}

/// How a history entry came to be recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The runner executed the body and recorded its completion.
    Executed,
    /// An operator asserted that an uncertain attempt's effects are complete;
    /// the runner never observed the body finish.
    Recovered,
}

impl Provenance {
    /// The name reports use.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::Recovered => "recovered",
        }
    }
}
