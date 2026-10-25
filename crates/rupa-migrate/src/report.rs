use crate::{Error, Version};

/// Successfully committed versions from one explicit invocation.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// Versions executed in actual installation order.
    pub applied: Vec<Version>,
    /// Interrupted transactional attempts found under exclusion and recorded
    /// as rolled back: completion commits with the body, so none committed.
    pub reconciled: Vec<i64>,
}

/// A run failed while retaining the typed body and infrastructure causes.
///
/// Every variant carries the versions this invocation committed before the
/// failure, so an operator can reconcile without re-reading history.
#[derive(Debug, thiserror::Error)]
pub enum RunError<E: std::error::Error + 'static, M: std::error::Error + 'static> {
    /// Catalog/history/session preparation or completion failed.
    #[error("{source}; applied: {applied:?}; cleanup errors: {cleanup:?}")]
    Infrastructure {
        /// Original infrastructure failure.
        #[source]
        source: Error<E>,
        /// Versions committed by this invocation before the failure, in installation order.
        applied: Box<[Version]>,
        /// Further rollback/release failures, never replacing the original error.
        cleanup: Vec<Error<E>>,
    },
    /// A user body failed; earlier migration commits remain intact.
    #[error(
        "migration {version} failed: {source}; applied: {applied:?}; cleanup errors: {cleanup:?}"
    )]
    Migration {
        /// The exact failed version.
        version: Version,
        /// Original typed application/migration error.
        #[source]
        source: M,
        /// Versions committed by this invocation before the failure, in installation order.
        applied: Box<[Version]>,
        /// Further rollback/release failures.
        cleanup: Vec<Error<E>>,
    },
}

impl<E: std::error::Error + 'static, M: std::error::Error + 'static> RunError<E, M> {
    /// Versions committed by this invocation before the failure, in installation order.
    pub fn applied(&self) -> &[Version] {
        match self {
            Self::Infrastructure { applied, .. } | Self::Migration { applied, .. } => applied,
        }
    }
}
