use rupa::migrate::{
    ApplyOptions, Blocker, Error, Init, Order, Policy, RunError, Version, VersionScheme,
};
use rupa::{Dialect, DialectId};

use super::{Connection, Harness, Session};
use crate::migrations::fixture::{Body, planned_with, reversible, snapshot, with_policy};

/// Permission to run late artifacts.
pub(super) const ALLOW_LATE: ApplyOptions = ApplyOptions {
    order: Order::AllowLate,
    ..ApplyOptions::DEFAULT
};

/// An apply through `text`, preferring a snapshot on a fresh target.
pub(super) fn through(text: &str) -> ApplyOptions {
    ApplyOptions {
        through: Some(version(text)),
        ..ApplyOptions::DEFAULT
    }
}

/// An apply through `text` that replays incrementals, never a snapshot.
pub(super) fn replay(text: &str) -> ApplyOptions {
    ApplyOptions {
        init: Init::FullReplay,
        ..through(text)
    }
}

/// A numbered version.
pub(super) fn version(text: &str) -> Version {
    VersionScheme::Numbered.parse(text).expect("version")
}

/// The versions history records, in installation order, after checking
/// that their ranks strictly increase.
pub(super) fn history<H: Harness>(h: &H) -> Vec<String> {
    let history = wait!(h, h.connect().history()).expect("history");

    assert!(
        history
            .windows(2)
            .all(|pair| pair[0].installation_rank < pair[1].installation_rank),
        "ranks follow installation order: {history:?}"
    );

    history.into_iter().map(|entry| entry.version).collect()
}

/// The blockers a refused apply or revert reports.
pub(super) fn blocked<E: std::error::Error, M: std::error::Error, R: std::fmt::Debug>(
    result: Result<impl Sized, RunError<E, M, R>>,
) -> Vec<Blocker> {
    match result {
        Err(RunError::Infrastructure {
            source: Error::Blocked(blockers),
            ..
        }) => blockers,
        Err(other) => panic!("expected blockers, got {other}"),
        Ok(_) => panic!("expected blockers, but the run went ahead"),
    }
}

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

/// A snapshot script fixture under the session's default policy.
pub(super) fn snapshot_of<H: Harness>(version: &'static str, sql: &'static str) -> Body {
    snapshot(version, sql, policy::<H>())
}

/// A reversible script fixture under the session's default policy.
pub(super) fn undoable<H: Harness>(
    version: &'static str,
    up: &'static str,
    down: &'static str,
) -> Body {
    reversible(version, up, down, policy::<H>())
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

/// Runs a body that panics on a dedicated session, which is then dropped
/// without cleanup, as when a runner process dies.
macro_rules! interrupt {
    ($h:expr, $catalog:expr) => {{
        let session = $h.connect();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = wait!($h, $catalog.apply(session));
        }));

        assert!(interrupted.is_err(), "the body panics");
    }};
}
