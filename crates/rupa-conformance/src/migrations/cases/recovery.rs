use rupa::migrate::{Error, Policy, Provenance, Recovery, RecoveryAction, RecoveryError, RunError};

use super::support::{busy, script, supports, transactional};
use super::{Apply, Harness, Recover, Session, catalog, versions};
use crate::migrations::fixture::{Interrupted, InterruptedAutocommit, with_policy};

/// The refusal a recovery result carries, if any.
fn refusal<E: std::error::Error>(result: Result<(), Error<E>>) -> Option<RecoveryError> {
    match result {
        Err(Error::Recovery(error)) => Some(error),
        _ => None,
    }
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

pub(super) fn case_interrupted_transactional_attempt_is_reconciled<H: Harness>(h: &mut H) {
    if !transactional::<H>() {
        return;
    }

    let interrupted = [Interrupted];
    let corrected = [script::<H>("1", "CREATE TABLE kept (id INTEGER);")];
    let corrected = catalog(&corrected);
    let release = Recovery::new(RecoveryAction::ReleaseClaim, "the runner process exited");

    interrupt!(h, catalog(&interrupted));

    let mut report = wait!(h, corrected.apply(h.connect()));

    if busy(&report) {
        wait!(h, corrected.recover(h.connect(), &release)).expect("the stale claim is released");
        report = wait!(h, corrected.apply(h.connect()));
    }

    let report = report.expect("apply after reconciliation");

    assert_eq!(
        report.reconciled.len(),
        1,
        "the rolled-back attempt is reported"
    );
    assert_eq!(versions(&report), ["1"], "a new attempt runs the artifact");
    assert!(!h.table_exists("lost"), "the interrupted body rolled back");
    assert!(h.table_exists("kept"));
    assert_eq!(
        h.count(
            "SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'rolled_back'"
        ),
        1
    );
}

pub(super) fn case_completed_recovery_records_without_running<H: Harness>(h: &mut H) {
    if !supports::<H>(Policy::NonTransactional) {
        return;
    }

    let interrupted = [InterruptedAutocommit];
    let interrupted = catalog(&interrupted);
    let changed = [with_policy(
        "1",
        "CREATE TABLE other (id INTEGER);",
        Policy::NonTransactional,
    )];
    let changed = catalog(&changed);

    interrupt!(h, interrupted);

    let attempt = match wait!(h, interrupted.apply(h.connect())) {
        Err(RunError::Infrastructure {
            source: Error::Uncertain(attempt),
            ..
        }) => attempt,
        other => panic!("the interrupted attempt blocks apply: {other:?}"),
    };
    let completed = Recovery::new(
        RecoveryAction::Completed(attempt),
        "survived exists as written",
    )
    .with_evidence("inspected the schema");

    assert_eq!(
        refusal(wait!(
            h,
            interrupted.recover(h.connect(), &Recovery::new(completed.action, " "))
        )),
        Some(RecoveryError::MissingReason)
    );
    assert_eq!(
        refusal(wait!(
            h,
            interrupted.recover(
                h.connect(),
                &Recovery::new(RecoveryAction::Completed(attempt + 100), "typo")
            )
        )),
        Some(RecoveryError::UnknownAttempt(attempt + 100))
    );
    assert!(matches!(
        refusal(wait!(h, changed.recover(h.connect(), &completed))),
        Some(RecoveryError::Changed { .. })
    ));

    wait!(h, interrupted.recover(h.connect(), &completed)).expect("completion is recorded");

    assert_eq!(
        refusal(wait!(h, interrupted.recover(h.connect(), &completed))),
        Some(RecoveryError::Resolved(attempt))
    );
    assert!(
        wait!(h, interrupted.apply(h.connect()))
            .expect("the recovered artifact is installed")
            .applied
            .is_empty(),
        "the body is never replayed"
    );

    let history = wait!(h, h.connect().history()).expect("history");

    assert_eq!(history.len(), 1);
    assert_eq!(history[0].provenance, Provenance::Recovered);
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'recovered' AND reason IS NOT NULL AND evidence IS NOT NULL"),
        1
    );
}

pub(super) fn case_restored_recovery_allows_a_new_apply<H: Harness>(h: &mut H) {
    if !supports::<H>(Policy::NonTransactional) {
        return;
    }

    let failing = [with_policy(
        "1",
        "CREATE TABLE partial (id INTEGER); INSERT INTO absent VALUES (1);",
        Policy::NonTransactional,
    )];
    let corrected = [with_policy(
        "1",
        "CREATE TABLE partial (id INTEGER);",
        Policy::NonTransactional,
    )];
    let corrected = catalog(&corrected);

    let attempt = match wait!(h, catalog(&failing).apply(h.connect())) {
        Err(RunError::Migration { cleanup, .. }) => match cleanup.as_slice() {
            [Error::Uncertain(attempt)] => *attempt,
            other => panic!("the failed body is uncertain: {other:?}"),
        },
        other => panic!("the body fails: {other:?}"),
    };

    h.execute("DROP TABLE partial");

    let restored = Recovery::new(RecoveryAction::Restored(attempt), "dropped partial by hand");

    wait!(h, corrected.recover(h.connect(), &restored)).expect("restoration is recorded");

    let report = wait!(h, corrected.apply(h.connect())).expect("a new apply runs the artifact");

    assert_eq!(versions(&report), ["1"]);
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'abandoned'"),
        1
    );
}

pub(super) fn case_recovery_requires_exclusion<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE owned (id INTEGER);")];
    let catalog = catalog(&entries);
    let restored = Recovery::new(RecoveryAction::Restored(1), "the owner is still running");

    wait!(h, catalog.apply(h.connect())).expect("initial apply");

    let mut owner = h.connect();

    wait!(
        h,
        owner.acquire("primary", rupa::migrate::VersionScheme::Numbered)
    )
    .expect("owner acquires");

    assert!(matches!(
        wait!(h, catalog.recover(h.connect(), &restored)),
        Err(Error::Busy)
    ));

    wait!(h, owner.release()).expect("owner releases");
}

pub(super) fn case_recovery_refuses_what_it_cannot_assert<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE asserted (id INTEGER);")];
    let catalog = catalog(&entries);
    let release = Recovery::new(RecoveryAction::ReleaseClaim, "no runner is left");
    let completed = Recovery::new(RecoveryAction::Completed(1), "nothing ran");

    assert_eq!(
        refusal(wait!(h, catalog.recover(h.connect(), &completed))),
        Some(RecoveryError::UnknownAttempt(1)),
        "a fresh target has no attempts"
    );
    assert!(
        !h.table_exists("_rupa_migrations"),
        "recovery never creates a journal"
    );

    wait!(h, catalog.apply(h.connect())).expect("apply");

    let applied = wait!(h, h.connect().history()).expect("history")[0].installation_rank as i64;
    let completed = Recovery::new(RecoveryAction::Completed(applied), "it ran");

    assert_eq!(
        refusal(wait!(h, catalog.recover(h.connect(), &release))),
        Some(RecoveryError::NoClaim)
    );
    assert_eq!(
        refusal(wait!(h, catalog.recover(h.connect(), &completed))),
        Some(RecoveryError::Resolved(applied)),
        "an applied attempt is resolved"
    );
}
