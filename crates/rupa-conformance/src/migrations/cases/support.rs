use rupa::core::dialect::{Dialect, DialectId};
use rupa::migrate::{Error, Policy, RunError};

use super::{Connection, Harness, Session};
use crate::migrations::fixture::{Body, planned_with, with_policy};

/// Whether the harness's sessions support `policy`.
pub(super) fn supports<H: Harness>(policy: Policy) -> bool {
    <H::Session as Session>::POLICIES.contains(&policy)
}

/// Whether bodies can run inside an artifact transaction, which the
/// rollback cases and the fixed transactional fixtures need.
pub(super) fn transactional<H: Harness>() -> bool {
    supports::<H>(Policy::TransactionalRequired)
}

/// The policy policy-agnostic cases declare: the session's first.
pub(super) fn policy<H: Harness>() -> Policy {
    <H::Session as Session>::POLICIES[0]
}

/// A script fixture under the session's default policy.
pub(super) fn script<H: Harness>(version: &'static str, sql: &'static str) -> Body {
    with_policy(version, sql, policy::<H>())
}

/// Planned fixture metadata under the session's default policy.
pub(super) fn planned<H: Harness>(version: &'static str) -> &'static rupa::migrate::Metadata {
    planned_with(version, policy::<H>())
}

/// The dialect the harness's sessions speak.
pub(super) fn dialect<H: Harness>(h: &H) -> DialectId {
    h.connect().dialect().id()
}

/// Whether a run found the target owned by another runner.
pub(super) fn busy<E: std::error::Error, M: std::error::Error>(
    result: &Result<impl Sized, RunError<E, M>>,
) -> bool {
    matches!(
        result,
        Err(RunError::Infrastructure {
            source: Error::Busy,
            ..
        })
    )
}
