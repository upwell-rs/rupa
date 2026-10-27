use rupa::migrate::{
    Adoption, AdoptionError, Blocker, CoverageSource, Error, Policy, Provenance, Recovery,
    RecoveryAction, RevertTarget, RunError,
};

use super::support::{
    ALLOW_LATE, blocked, history, replay, script, snapshot_of, supports, through, transactional,
    undoable, version,
};
use super::{Adopt, Apply, Harness, Inspect, Recover, Revert, catalog, versions};
use crate::migrations::fixture::{Body, snapshot, with_policy};

/// Incrementals `1` to `5`, each creating its table.
fn chain<H: Harness>() -> [Body; 5] {
    [
        script::<H>("1", "CREATE TABLE t1 (id INTEGER);"),
        script::<H>("2", "CREATE TABLE t2 (id INTEGER);"),
        script::<H>("3", "CREATE TABLE t3 (id INTEGER);"),
        script::<H>("4", "CREATE TABLE t4 (id INTEGER);"),
        script::<H>("5", "CREATE TABLE t5 (id INTEGER);"),
    ]
}

/// A snapshot at `2`, creating what `1` and `2` create.
fn at_two<H: Harness>() -> Body {
    snapshot_of::<H>(
        "2",
        "CREATE TABLE t1 (id INTEGER); CREATE TABLE t2 (id INTEGER);",
    )
}

/// A snapshot at `4`, creating what `1` to `4` create.
fn at_four<H: Harness>() -> Body {
    snapshot_of::<H>(
        "4",
        "CREATE TABLE t1 (id INTEGER); CREATE TABLE t2 (id INTEGER); CREATE TABLE t3 (id INTEGER); CREATE TABLE t4 (id INTEGER);",
    )
}

pub(super) fn case_a_fresh_target_runs_the_snapshot_then_the_tail<H: Harness>(h: &mut H) {
    let [one, two, three, ..] = chain::<H>();
    let entries = [one, two, three, at_two::<H>()];
    let catalog = catalog(&entries);

    let plan = wait!(h, catalog.plan(h.connect())).expect("plan");
    let report = wait!(h, catalog.apply(h.connect())).expect("apply");
    let status = wait!(h, catalog.status(h.connect())).expect("status");
    let again = wait!(h, catalog.apply(h.connect())).expect("apply again");

    assert_eq!(plan.covered, [version("1"), version("2")]);
    assert_eq!(report.snapshot, Some(version("2")));
    assert_eq!(report.covered, [version("1"), version("2")]);
    assert_eq!(versions(&report), ["3"], "covered bodies never run");
    assert_eq!(
        history(h),
        ["3"],
        "covered incrementals are not installations"
    );
    assert!(
        matches!(
            &status.coverage,
            Some(coverage) if coverage.boundary == "2"
                && coverage.members.len() == 2
                && matches!(coverage.source, CoverageSource::Snapshot { provenance: Provenance::Executed, .. })
        ),
        "{status:?}"
    );
    assert!(status.blockers.is_empty() && status.pending.is_empty());
    assert!(["t1", "t2", "t3"].iter().all(|table| h.table_exists(table)));
    assert!(again.snapshot.is_none() && again.applied.is_empty());
}

pub(super) fn case_the_target_version_selects_the_snapshot_and_replay_skips_it<H: Harness>(
    h: &mut H,
) {
    let [one, two, three, four, five] = chain::<H>();
    let both = [one, two, three, four, five, at_two::<H>(), at_four::<H>()];
    let only_four = [one, two, three, four, at_four::<H>()];

    let below_four =
        wait!(h, catalog(&both).apply_with(h.connect(), &through("3"))).expect("apply");

    assert_eq!(
        below_four.snapshot,
        Some(version("2")),
        "snapshot 4 is above the target"
    );
    assert_eq!(versions(&below_four), ["3"]);
    assert!(h.table_exists("t3") && !h.table_exists("t4"));

    h.reset();

    let none_eligible = wait!(
        h,
        catalog(&only_four).apply_with(h.connect(), &through("3"))
    )
    .expect("apply");

    assert!(none_eligible.snapshot.is_none());
    assert_eq!(versions(&none_eligible), ["1", "2", "3"]);

    h.reset();

    let replayed = wait!(h, catalog(&both).apply_with(h.connect(), &replay("3"))).expect("replay");

    assert!(replayed.snapshot.is_none() && replayed.covered.is_empty());
    assert_eq!(versions(&replayed), ["1", "2", "3"]);
    assert_eq!(history(h), ["1", "2", "3"]);
}

pub(super) fn case_a_managed_target_never_runs_a_newer_snapshot<H: Harness>(h: &mut H) {
    let [one, two, three, ..] = chain::<H>();
    let with_snapshot = [
        one,
        two,
        three,
        snapshot_of::<H>(
            "3",
            "CREATE TABLE t1 (id INTEGER); CREATE TABLE t2 (id INTEGER); CREATE TABLE t3 (id INTEGER);",
        ),
    ];

    let with_snapshot = catalog(&with_snapshot);

    wait!(h, catalog(&[one, two]).apply(h.connect())).expect("apply 1 and 2");

    let report = wait!(h, with_snapshot.apply(h.connect())).expect("apply 3");
    let status = wait!(h, with_snapshot.status(h.connect())).expect("status");

    assert!(report.snapshot.is_none());
    assert_eq!(versions(&report), ["3"]);
    assert!(status.coverage.is_none());
    assert_eq!(history(h), ["1", "2", "3"]);
}

pub(super) fn case_only_a_never_initialized_journal_starts_from_a_snapshot<H: Harness>(h: &mut H) {
    let entries = [
        undoable::<H>("1", "CREATE TABLE t1 (id INTEGER);", "DROP TABLE t1;"),
        undoable::<H>("2", "CREATE TABLE t2 (id INTEGER);", "DROP TABLE t2;"),
        at_two::<H>(),
    ];
    let stack = catalog(&entries);

    wait!(h, stack.apply_with(h.connect(), &replay("2"))).expect("replay 1 and 2");
    wait!(h, stack.revert(h.connect(), RevertTarget::Last(2))).expect("revert both");

    let reapplied = wait!(h, stack.apply(h.connect())).expect("apply again");

    assert!(
        reapplied.snapshot.is_none(),
        "a target whose installations were reverted is not fresh"
    );
    assert_eq!(versions(&reapplied), ["1", "2"]);

    if !transactional::<H>() {
        return;
    }

    let failing = [
        entries[0],
        entries[1],
        snapshot_of::<H>(
            "2",
            "CREATE TABLE t1 (id INTEGER); INSERT INTO absent VALUES (1);",
        ),
    ];

    h.reset();
    wait!(h, catalog(&failing).apply(h.connect())).expect_err("the snapshot body fails");

    let retried = wait!(h, stack.apply(h.connect())).expect("apply the corrected snapshot");

    assert_eq!(
        retried.snapshot,
        Some(version("2")),
        "a rolled-back first attempt leaves the journal uninitialized"
    );
}

pub(super) fn case_snapshot_and_replay_reach_the_same_state<H: Harness>(h: &mut H) {
    let entries = [
        script::<H>(
            "1",
            "CREATE TABLE t1 (id INTEGER); INSERT INTO t1 VALUES (1);",
        ),
        script::<H>(
            "2",
            "INSERT INTO t1 VALUES (2); CREATE TABLE t2 (id INTEGER);",
        ),
        script::<H>("3", "INSERT INTO t2 SELECT id FROM t1;"),
        snapshot_of::<H>(
            "2",
            "CREATE TABLE t1 (id INTEGER); INSERT INTO t1 VALUES (1); INSERT INTO t1 VALUES (2); CREATE TABLE t2 (id INTEGER);",
        ),
    ];
    let state = |h: &H| {
        [
            h.count("SELECT count(*) FROM t1"),
            h.count("SELECT coalesce(sum(id), 0) FROM t1"),
            h.count("SELECT count(*) FROM t2"),
            h.count("SELECT coalesce(sum(id), 0) FROM t2"),
        ]
    };

    wait!(h, catalog(&entries).apply(h.connect())).expect("snapshot and tail");

    let snapshotted = state(h);

    h.reset();
    wait!(h, catalog(&entries).apply_with(h.connect(), &replay("3"))).expect("full replay");

    assert_eq!(snapshotted, [2, 3, 2, 3]);
    assert_eq!(
        state(h),
        snapshotted,
        "both paths reach the same owned state"
    );
}

pub(super) fn case_work_added_at_or_below_a_snapshot_is_uncovered<H: Harness>(h: &mut H) {
    let [one, two, three, ..] = chain::<H>();
    let gap = snapshot_of::<H>(
        "3",
        "CREATE TABLE t1 (id INTEGER); CREATE TABLE t3 (id INTEGER);",
    );
    let uncovered = |blockers: &[Blocker], missing: &str| {
        blockers.iter().any(|blocker| {
            matches!(blocker, Blocker::Uncovered { version: uncovered, boundary }
                if *uncovered == version(missing) && *boundary == version("3"))
        })
    };

    wait!(h, catalog(&[one, three, gap]).apply(h.connect())).expect("snapshot over 1 and 3");

    let late = blocked(wait!(
        h,
        catalog(&[one, two, three, gap]).apply_with(h.connect(), &ALLOW_LATE)
    ));

    assert!(
        uncovered(&late, "2"),
        "late permission never covers 2: {late:?}"
    );
    assert!(!h.table_exists("t2"));

    h.reset();
    wait!(h, catalog(&[one, two, gap]).apply(h.connect())).expect("snapshot over 1 and 2");

    let at_boundary = blocked(wait!(
        h,
        catalog(&[one, two, three, gap]).apply(h.connect())
    ));

    assert!(
        uncovered(&at_boundary, "3"),
        "an artifact at the boundary is not part of the tail: {at_boundary:?}"
    );
}

pub(super) fn case_covered_sources_must_stay_unchanged_and_present<H: Harness>(h: &mut H) {
    let [one, two, three, ..] = chain::<H>();
    let edited = script::<H>("1", "CREATE TABLE edited (id INTEGER);");

    wait!(
        h,
        catalog(&[one, two, three, at_two::<H>()]).apply(h.connect())
    )
    .expect("apply");

    let changed = blocked(wait!(
        h,
        catalog(&[edited, two, three, at_two::<H>()]).apply(h.connect())
    ));
    let removed = blocked(wait!(h, catalog(&[one, two, three]).apply(h.connect())));

    assert!(
        changed.iter().any(|blocker| matches!(
            blocker,
            Blocker::Changed { version: changed, .. } if *changed == version("1")
        )),
        "{changed:?}"
    );
    assert_eq!(
        removed.len(),
        1,
        "only the executed snapshot is missing: {removed:?}"
    );
    assert!(matches!(&removed[0], Blocker::Missing { version, .. } if version == "2"));
}

pub(super) fn case_a_revert_stops_at_the_snapshot_boundary<H: Harness>(h: &mut H) {
    let [one, two, ..] = chain::<H>();
    let entries = [
        one,
        two,
        at_two::<H>(),
        undoable::<H>("3", "CREATE TABLE t3 (id INTEGER);", "DROP TABLE t3;"),
    ];
    let stack = catalog(&entries);

    wait!(h, stack.apply(h.connect())).expect("snapshot and tail");

    let below = blocked(wait!(
        h,
        stack.revert(h.connect(), RevertTarget::To(version("1")))
    ));
    let beyond = blocked(wait!(h, stack.revert(h.connect(), RevertTarget::Last(2))));
    let report =
        wait!(h, stack.revert(h.connect(), RevertTarget::To(version("2")))).expect("revert to 2");
    let status = wait!(h, stack.status(h.connect())).expect("status");
    let reapplied = wait!(h, stack.apply(h.connect())).expect("reapply the tail");

    assert_eq!(
        below,
        [Blocker::Boundary {
            requested: version("1"),
            boundary: version("2"),
        }]
    );
    assert_eq!(
        beyond,
        [Blocker::Exceeds {
            requested: 2,
            installed: 1,
        }]
    );
    assert_eq!(report.reverted, [version("3")]);
    assert!(report.remaining.is_empty());
    assert!(
        status.coverage.is_some(),
        "reaching the boundary keeps the coverage"
    );
    assert!(
        reapplied.snapshot.is_none(),
        "a covered target is not fresh again"
    );
    assert_eq!(versions(&reapplied), ["3"]);
}

pub(super) fn case_an_uncertain_snapshot_is_recovered_never_replayed<H: Harness>(h: &mut H) {
    if !supports::<H>(Policy::NonTransactional) {
        return;
    }

    let entries = [
        with_policy(
            "1",
            "CREATE TABLE t1 (id INTEGER);",
            Policy::NonTransactional,
        ),
        snapshot(
            "1",
            "CREATE TABLE t1 (id INTEGER); INSERT INTO absent VALUES (1);",
            Policy::NonTransactional,
        ),
    ];
    let partial = catalog(&entries);

    wait!(h, partial.apply(h.connect())).expect_err("the snapshot body fails");

    let status = wait!(h, partial.status(h.connect())).expect("status");
    let [attempt] = status.attempts.as_slice() else {
        panic!("one unresolved attempt: {status:?}");
    };
    let blind = wait!(h, partial.apply(h.connect()));
    let adopted = wait!(
        h,
        partial.adopt(h.connect(), &Adoption::new(version("1"), "try to skip it"))
    );
    let completed = Recovery::new(RecoveryAction::Completed(attempt.id), "the table exists");

    assert_eq!(attempt.kind, rupa::migrate::MigrationKind::Snapshot);
    assert!(
        matches!(
            blind,
            Err(RunError::Infrastructure {
                source: Error::Uncertain(_),
                ..
            })
        ),
        "{blind:?}"
    );
    assert!(
        matches!(adopted, Err(Error::Adoption(AdoptionError::Managed))),
        "{adopted:?}"
    );

    wait!(h, partial.recover(h.connect(), &completed)).expect("assert the snapshot complete");

    let recovered = wait!(h, partial.status(h.connect())).expect("status");

    assert!(
        matches!(
            &recovered.coverage,
            Some(coverage) if matches!(coverage.source, CoverageSource::Snapshot { provenance: Provenance::Recovered, .. })
                && coverage.members.len() == 1
        ),
        "{recovered:?}"
    );
    assert!(recovered.blockers.is_empty() && recovered.pending.is_empty());
}
