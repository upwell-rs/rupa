use crate::{Error, Version};

/// Successfully committed versions from one explicit invocation.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// The snapshot executed to initialize a fresh target, if one was.
    pub snapshot: Option<Version>,
    /// The incrementals that snapshot covers, in version order. None of
    /// their bodies ran.
    pub covered: Vec<Version>,
    /// Incremental versions executed in actual installation order.
    pub applied: Vec<Version>,
    /// The applied versions that ran late, below an installed version, by
    /// explicit [`Order::AllowLate`](crate::Order::AllowLate) permission.
    pub late: Vec<Version>,
}

/// The installations one explicit revert undid.
#[derive(Debug, Default)]
pub struct RevertReport {
    /// Versions undone, in the order they ran: newest installation first.
    pub reverted: Vec<Version>,
    /// The versions still installed, in installation order. On a failed
    /// revert this is the active stack the failure left, including a revert
    /// refused before any undo; it is empty only if the run failed before
    /// reading history.
    pub remaining: Vec<Version>,
}

/// A run failed while retaining the typed body and infrastructure causes.
///
/// Every variant carries the report `R` of what this invocation committed
/// before the failure, so an operator can reconcile without re-reading
/// history: an [`ApplyReport`], late versions included, or a [`RevertReport`].
#[derive(Debug, thiserror::Error)]
pub enum RunError<
    E: std::error::Error + 'static,
    M: std::error::Error + 'static,
    R: std::fmt::Debug = ApplyReport,
> {
    /// Catalog/history/session preparation or completion failed.
    #[error("{source}; committed: {report:?}; cleanup errors: {cleanup:?}")]
    Infrastructure {
        /// Original infrastructure failure.
        #[source]
        source: Error<E>,
        /// What this invocation committed before the failure.
        report: Box<R>,
        /// Further rollback/release failures, never replacing the original error.
        cleanup: Vec<Error<E>>,
    },
    /// A user body failed; earlier migration commits remain intact.
    #[error(
        "migration {version} failed: {source}; committed: {report:?}; cleanup errors: {cleanup:?}"
    )]
    Migration {
        /// The exact failed version.
        version: Version,
        /// Original typed application/migration error.
        #[source]
        source: M,
        /// What this invocation committed before the failure.
        report: Box<R>,
        /// Further rollback/release failures.
        cleanup: Vec<Error<E>>,
    },
}

impl<E: std::error::Error + 'static, M: std::error::Error + 'static, R: std::fmt::Debug>
    RunError<E, M, R>
{
    /// What this invocation committed before the failure.
    pub fn report(&self) -> &R {
        match self {
            Self::Infrastructure { report, .. } | Self::Migration { report, .. } => report,
        }
    }
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
}
