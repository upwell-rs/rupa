use crate::{Checksum, ValidationError};

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
}

/// Durable successfully executed incremental history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Strictly increasing installation order, separate from version order.
    pub installation_rank: u64,
    /// Canonical logical version.
    pub version: String,
    /// Checksum of the installed source closure.
    pub checksum: Checksum,
}
