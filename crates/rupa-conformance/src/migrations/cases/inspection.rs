use std::time::{Duration, Instant};

use rupa::migrate::{
    Blocker, Catalog, Classification, Exclusion, Execution, Plan, Planned, Policy, Provenance,
    Recovery, RecoveryAction, RunError, Status, VersionScheme,
};

use super::support::{planned, script, supports, transactional};
use super::{Apply, Harness, Inspect, Recover, Session, catalog};
use crate::migrations::fixture::{Body, Interrupted, InterruptedAutocommit, Plans};

/// The journal's rows, which inspection must leave unchanged.
fn rows<H: Harness>(h: &H) -> i64 {
    h.count("SELECT count(*) FROM _rupa_migrations")
}

/// The journal's running attempts, which inspection must never resolve.
fn running<H: Harness>(h: &H) -> i64 {
    h.count("SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'running'")
}

fn pending(status: &Status<'_>) -> Vec<String> {
    status
        .pending
        .iter()
        .map(|entry| entry.version.to_string())
        .collect()
}

/// The status once the server has ended an interrupted runner's session.
/// It does so asynchronously after the connection closed, and holds the
/// runner's lock until then, so this reads again while the target is locked.
fn settled<'c, H: Harness>(h: &H, catalog: &'c Catalog<'_, Body>) -> Status<'c> {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let status = wait!(h, catalog.status(h.connect())).expect("status");

        if status.exclusion != Exclusion::Locked || Instant::now() > deadline {
            return status;
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn case_inspection_never_initializes_a_target<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE inspected (id INTEGER);")];
    let catalog = catalog(&entries);

    let fresh = wait!(h, catalog.status(h.connect())).expect("status of a fresh target");
    let plan = wait!(h, catalog.plan(h.connect())).expect("plan of a fresh target");

    assert_eq!(fresh.classification, Classification::Fresh);
    assert_eq!(fresh.exclusion, Exclusion::Free);
    assert_eq!(pending(&fresh), ["1"]);
    assert!(fresh.blockers.is_empty());
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        plan.steps[0].execution,
        Execution::Opaque,
        "a Rust body is opaque"
    );
    assert!(
        !h.table_exists("_rupa_migrations"),
        "inspection never creates a journal"
    );
    assert!(!h.table_exists("inspected"), "inspection never runs a body");

    h.execute("CREATE TABLE stray (id INTEGER)");

    let unmanaged = wait!(h, catalog.status(h.connect())).expect("status of an unmanaged target");

    assert_eq!(unmanaged.classification, Classification::Unmanaged);
    assert_eq!(unmanaged.blockers, [Blocker::Unmanaged]);
    assert!(!h.table_exists("_rupa_migrations"));
}

pub(super) fn case_status_and_plan_report_without_writing<H: Harness>(h: &mut H) {
    let installed = [script::<H>("1", "CREATE TABLE first (id INTEGER);")];
    let entries = [
        installed[0],
        script::<H>("2", "CREATE TABLE second (id INTEGER);"),
    ];
    let catalog = catalog(&entries);
    let planned = [Planned(Plans(planned::<H>("1"), || {
        Plan::new().sql("CREATE TABLE planned (id INTEGER)")
    }))];
    let planned = super::catalog(&planned);

    wait!(h, super::catalog(&installed).apply(h.connect())).expect("apply the first entry");

    let journal = rows(h);
    let status = wait!(h, catalog.status(h.connect())).expect("status");
    let plan = wait!(h, catalog.plan(h.connect())).expect("plan");

    assert_eq!(status.classification, Classification::Managed);
    assert_eq!(status.history.len(), 1);
    assert_eq!(status.history[0].version, "1");
    assert_eq!(status.history[0].provenance, Provenance::Executed);
    assert_eq!(pending(&status), ["2"]);
    assert!(status.blockers.is_empty());
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.entry.version.to_string())
            .collect::<Vec<_>>(),
        ["2"]
    );
    assert_eq!(rows(h), journal, "inspection writes nothing");
    assert!(!h.table_exists("second"), "inspection never runs a body");

    h.reset();

    let plan = wait!(h, planned.plan(h.connect())).expect("plan of a planned catalog");

    assert_eq!(
        plan.steps[0].execution,
        Execution::Static(vec!["CREATE TABLE planned (id INTEGER)".into()]),
        "a planned body is its exact scripts"
    );
    assert!(!h.table_exists("planned"));
}

pub(super) fn case_status_sees_a_live_runner_without_waiting<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE owned (id INTEGER);")];
    let catalog = catalog(&entries);

    wait!(h, catalog.apply(h.connect())).expect("initial apply");

    let mut owner = h.connect();

    wait!(h, owner.acquire("primary", VersionScheme::Numbered)).expect("owner acquires");

    let started = Instant::now();
    let held = wait!(h, catalog.status(h.connect())).expect("status beside a live runner");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "status does not wait for exclusion"
    );
    assert!(
        matches!(
            (&held.exclusion, held.blockers.as_slice()),
            (Exclusion::Locked, [Blocker::Locked]) | (Exclusion::Claimed, [Blocker::Claimed])
        ),
        "the live runner blocks apply: {held:?}"
    );

    wait!(h, owner.release()).expect("owner releases");

    let free = wait!(h, catalog.status(h.connect())).expect("status after release");

    assert_eq!(free.exclusion, Exclusion::Free);
    assert_eq!(free.classification, Classification::Managed);
}

pub(super) fn case_status_reports_attempts_and_leaves_them_running<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE corrected (id INTEGER);")];
    let catalog = catalog(&entries);
    let release = Recovery::new(RecoveryAction::ReleaseClaim, "the runner process exited");

    if transactional::<H>() {
        interrupt!(h, super::catalog(&[Interrupted]));

        let mut status = settled(h, &catalog);

        if status.exclusion == Exclusion::Claimed {
            assert_eq!(status.blockers, [Blocker::Claimed]);

            wait!(h, catalog.recover(h.connect(), &release)).expect("the stale claim is released");
            status = settled(h, &catalog);
        }

        assert_eq!(status.classification, Classification::Uncertain);
        assert!(
            matches!(
                status.blockers.as_slice(),
                [Blocker::Interrupted(attempt)]
                    if attempt.version == "1" && attempt.policy == Policy::TransactionalRequired
            ),
            "the interrupted attempt is reported: {status:?}"
        );
        assert_eq!(running(h), 1, "inspection never reconciles");
        assert!(
            matches!(
                wait!(h, catalog.apply(h.connect())),
                Err(RunError::Infrastructure {
                    source: rupa::migrate::Error::Reconciled(_),
                    ..
                })
            ),
            "the attempt was left for apply to reconcile"
        );

        h.reset();
    }

    if supports::<H>(Policy::NonTransactional) {
        interrupt!(h, super::catalog(&[InterruptedAutocommit]));

        let status = settled(h, &catalog);

        assert!(
            matches!(
                status.blockers.as_slice(),
                [Blocker::Unresolved(attempt)]
                    if attempt.version == "1" && attempt.policy == Policy::NonTransactional
            ),
            "the unresolved attempt is reported: {status:?}"
        );
        assert_eq!(status.attempts.len(), 1);
        assert_eq!(running(h), 1, "inspection never resolves an attempt");
    }
}

pub(super) fn case_status_reports_every_history_mismatch<H: Harness>(h: &mut H) {
    let installed = [
        script::<H>("1", "CREATE TABLE first (id INTEGER);"),
        script::<H>("3", "CREATE TABLE third (id INTEGER);"),
    ];
    let edited = [
        script::<H>("1", "CREATE TABLE edited (id INTEGER);"),
        script::<H>("2", "CREATE TABLE late (id INTEGER);"),
    ];
    let edited = catalog(&edited);
    let other = Catalog::new("other", VersionScheme::Numbered, &installed).expect("valid catalog");

    wait!(h, catalog(&installed).apply(h.connect())).expect("apply");

    let checksum = |version: &str| {
        wait!(h, h.connect().history())
            .expect("history")
            .into_iter()
            .find(|entry| entry.version == version)
            .expect("installed")
            .checksum
    };
    let version = |text: &str| VersionScheme::Numbered.parse(text).expect("version");
    let status = wait!(h, edited.status(h.connect())).expect("status");
    let entry = |text: &str| {
        edited
            .entries()
            .find(|entry| entry.version.to_string() == text)
            .expect("catalog entry")
            .checksum
    };

    assert_eq!(
        status.blockers,
        [
            Blocker::Changed {
                version: version("1"),
                installed: checksum("1"),
                catalog: entry("1"),
            },
            Blocker::Missing {
                version: "3".into(),
                checksum: checksum("3"),
            },
            Blocker::Late {
                version: version("2"),
                checksum: entry("2"),
            },
        ]
    );
    assert!(matches!(
        wait!(h, other.status(h.connect()))
            .expect("status of another stream")
            .blockers
            .as_slice(),
        [Blocker::Stream { stream, .. }] if stream == "primary"
    ));
}
