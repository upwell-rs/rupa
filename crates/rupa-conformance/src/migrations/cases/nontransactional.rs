use rupa::migrate::{Error, Policy, RunError};

use super::support::{script, transactional};
use super::{Apply, Harness, Session, catalog, versions};
use crate::migrations::fixture::{InterruptedAutocommit, body, with_policy};

/// Whether the harness's sessions run nontransactional bodies; the cases
/// below pass vacuously for engines that reject the policy before bootstrap.
fn supported<H: Harness>() -> bool {
    super::support::supports::<H>(Policy::NonTransactional)
}

fn uncertain<T, E: std::error::Error, M: std::error::Error>(
    result: &Result<T, RunError<E, M>>,
) -> bool {
    matches!(
        result,
        Err(RunError::Infrastructure {
            source: Error::Uncertain(_),
            ..
        })
    )
}

/// Contrasts both policies, so it needs an engine supporting both.
pub(super) fn case_policies_apply_in_catalog_order_once<H: Harness>(h: &mut H) {
    if !supported::<H>() || !transactional::<H>() {
        return;
    }

    let entries = [
        body("1", "CREATE TABLE events (id INTEGER);"),
        with_policy(
            "2",
            "INSERT INTO events VALUES (1); CREATE TABLE audit (id INTEGER);",
            Policy::NonTransactional,
        ),
        body("3", "INSERT INTO audit SELECT id FROM events;"),
    ];
    let catalog = catalog(&entries);

    let mut session = h.connect();
    let report = wait!(h, session.apply(&catalog)).expect("mixed policy apply");

    assert_eq!(versions(&report), ["1", "2", "3"]);
    assert!(
        wait!(h, session.apply(&catalog))
            .expect("idempotent apply")
            .applied
            .is_empty()
    );
    assert_eq!(h.count("SELECT count(*) FROM audit"), 1);
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'applied'"),
        3
    );
}

pub(super) fn case_failed_body_keeps_effects_and_blocks_replay<H: Harness>(h: &mut H) {
    if !supported::<H>() {
        return;
    }

    let entries = [
        script::<H>("1", "CREATE TABLE kept (id INTEGER);"),
        with_policy(
            "2",
            "CREATE TABLE partial (id INTEGER); INSERT INTO absent VALUES (2);",
            Policy::NonTransactional,
        ),
    ];
    let catalog = catalog(&entries);

    let mut session = h.connect();
    let failed = wait!(h, session.apply(&catalog)).expect_err("second body fails");
    let replay = wait!(h, h.connect().apply(&catalog));

    match &failed {
        RunError::Migration {
            version, cleanup, ..
        } => {
            assert_eq!(version.to_string(), "2");
            assert!(
                matches!(cleanup.as_slice(), [Error::Uncertain(_)]),
                "the failure reports its unresolved attempt: {cleanup:?}"
            );
        }
        other => panic!("the body failure must be reported as such: {other}"),
    }

    assert_eq!(
        failed
            .applied()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["1"]
    );
    assert!(
        uncertain(&replay),
        "the lock was released, replay is refused"
    );
    assert!(
        h.table_exists("partial"),
        "earlier statements stay committed"
    );
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'"),
        1
    );
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'failed'"),
        0
    );
}

pub(super) fn case_interrupted_body_is_uncertain<H: Harness>(h: &mut H) {
    if !supported::<H>() {
        return;
    }

    let entries = [InterruptedAutocommit];
    let catalog = catalog(&entries);

    let session = h.connect();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wait!(h, catalog.apply(session));
    }));
    let competitor = wait!(h, h.connect().apply(&catalog));

    assert!(interrupted.is_err());
    assert!(
        matches!(
            competitor,
            Err(RunError::Infrastructure {
                source: Error::Busy | Error::Uncertain(_),
                ..
            })
        ),
        "an interrupted run blocks blind replay"
    );
    assert!(
        h.table_exists("survived"),
        "nontransactional effects persist"
    );
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'"),
        1
    );
}

/// Contrasts both policies, so it needs an engine supporting both.
pub(super) fn case_statements_outside_transactions_need_the_policy<H: Harness>(h: &mut H) {
    if !supported::<H>() || !transactional::<H>() {
        return;
    }

    let script = h.outside_transaction_script();
    let notes = body("1", "CREATE TABLE notes (id INTEGER, title TEXT);");
    let transactional = [notes, body("2", script)];
    let nontransactional = [notes, with_policy("2", script, Policy::NonTransactional)];

    let mut session = h.connect();
    let refused = wait!(h, session.apply(&catalog(&transactional)));
    let report = wait!(h, session.apply(&catalog(&nontransactional))).expect("autocommit apply");

    assert!(
        matches!(&refused, Err(RunError::Migration { version, .. }) if version.to_string() == "2"),
        "the engine refuses these statements inside the artifact transaction"
    );
    assert_eq!(versions(&report), ["2"]);
    assert_eq!(h.count("SELECT count(*) FROM _rupa_migration_history"), 2);
}

pub(super) fn case_body_refused_before_dispatch_fails_without_uncertainty<H: Harness>(h: &mut H) {
    if !supported::<H>() {
        return;
    }

    let refused = [with_policy(
        "1",
        "CREATE TABLE never (id INTEGER); COMMIT;",
        Policy::NonTransactional,
    )];
    let corrected = [with_policy(
        "1",
        "CREATE TABLE corrected (id INTEGER);",
        Policy::NonTransactional,
    )];

    let mut session = h.connect();
    let failed = wait!(h, session.apply(&catalog(&refused))).expect_err("refused script");
    let report = wait!(h, session.apply(&catalog(&corrected))).expect("corrected apply");

    match &failed {
        RunError::Migration { cleanup, .. } => {
            assert!(
                cleanup.is_empty(),
                "nothing ran, nothing is uncertain: {cleanup:?}"
            )
        }
        other => panic!("the refused body must be reported as such: {other}"),
    }

    assert_eq!(versions(&report), ["1"]);
    assert!(!h.table_exists("never"));
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'failed'"),
        1
    );
}
