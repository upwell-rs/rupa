/// An operator's assertion about an interrupted run. Recovery records it with
/// its reason and evidence in the audit events; it never executes a body.
#[derive(Debug, Clone, Copy)]
pub struct Recovery<'a> {
    /// What the operator asserts.
    pub action: RecoveryAction,
    /// Why the assertion holds. Required.
    pub reason: &'a str,
    /// Supporting evidence, such as a ticket, log or backup reference.
    pub evidence: Option<&'a str>,
}

impl<'a> Recovery<'a> {
    /// Asserts `action` for `reason`, without evidence.
    pub const fn new(action: RecoveryAction, reason: &'a str) -> Self {
        Self {
            action,
            reason,
            evidence: None,
        }
    }

    /// Adds supporting evidence.
    pub const fn with_evidence(mut self, evidence: &'a str) -> Self {
        self.evidence = Some(evidence);

        self
    }
}

/// What an operator asserts about an interrupted run, after establishing
/// that the runner which left it has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// The effects of unresolved attempt `0` are complete: its artifact is
    /// recorded as installed with [`Provenance::Recovered`](crate::Provenance::Recovered).
    /// The catalog must hold the checksum the attempt ran.
    Completed(i64),
    /// The effects of unresolved attempt `0` were manually undone: the
    /// attempt is abandoned, and only a new explicit apply runs its artifact.
    Restored(i64),
    /// The runner that left SQLite's durable owner claim has stopped: the
    /// claim is cleared. Unresolved attempts stay unresolved.
    ReleaseClaim,
}

/// Why a recovery assertion was refused. Nothing was recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecoveryError {
    /// The assertion has no reason.
    #[error("recovery requires an audit reason")]
    MissingReason,
    /// The target has no such attempt.
    #[error("migration attempt {0} does not exist")]
    UnknownAttempt(i64),
    /// The attempt is not unresolved, so there is nothing to assert.
    #[error("migration attempt {0} is already resolved")]
    Resolved(i64),
    /// The attempt ran a version the catalog does not contain.
    #[error(
        "migration attempt {attempt} ran version {version}, which the catalog does not contain"
    )]
    Missing {
        /// The attempt.
        attempt: i64,
        /// The version it ran.
        version: String,
    },
    /// The attempt ran a different checksum than the catalog's artifact.
    #[error(
        "migration attempt {attempt} ran a different checksum of version {version} than the catalog's"
    )]
    Changed {
        /// The attempt.
        attempt: i64,
        /// The version it ran.
        version: String,
    },
    /// The target holds no owner claim to release.
    #[error("the migration target holds no owner claim")]
    NoClaim,
}
