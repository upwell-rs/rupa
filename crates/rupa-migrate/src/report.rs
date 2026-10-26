use crate::{Error, Version};

/// Successfully committed versions from one explicit invocation.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// Versions executed in actual installation order.
    pub applied: Vec<Version>,
    /// The applied versions that ran late, below an installed version, by
    /// explicit [`Order::AllowLate`](crate::Order::AllowLate) permission.
    pub late: Vec<Version>,
}

/// A run failed while retaining the typed body and infrastructure causes.
///
/// Every variant carries the report of what this invocation committed before
/// the failure, late versions included, so an operator can reconcile without
/// re-reading history.
#[derive(Debug, thiserror::Error)]
pub enum RunError<E: std::error::Error + 'static, M: std::error::Error + 'static> {
    /// Catalog/history/session preparation or completion failed.
    #[error("{source}; applied: {:?}; cleanup errors: {cleanup:?}", report.applied)]
    Infrastructure {
        /// Original infrastructure failure.
        #[source]
        source: Error<E>,
        /// What this invocation committed before the failure.
        report: Box<ApplyReport>,
        /// Further rollback/release failures, never replacing the original error.
        cleanup: Vec<Error<E>>,
    },
    /// A user body failed; earlier migration commits remain intact.
    #[error(
        "migration {version} failed: {source}; applied: {:?}; cleanup errors: {cleanup:?}",
        report.applied
    )]
    Migration {
        /// The exact failed version.
        version: Version,
        /// Original typed application/migration error.
        #[source]
        source: M,
        /// What this invocation committed before the failure.
        report: Box<ApplyReport>,
        /// Further rollback/release failures.
        cleanup: Vec<Error<E>>,
    },
}

impl<E: std::error::Error + 'static, M: std::error::Error + 'static> RunError<E, M> {
    /// Versions committed by this invocation before the failure, in installation order.
    pub fn applied(&self) -> &[Version] {
        &self.report().applied
    }

    /// The committed versions that ran late, by explicit permission.
    pub fn late(&self) -> &[Version] {
        &self.report().late
    }

    /// What this invocation committed before the failure.
    pub fn report(&self) -> &ApplyReport {
        match self {
            Self::Infrastructure { report, .. } | Self::Migration { report, .. } => report,
        }
    }
}
