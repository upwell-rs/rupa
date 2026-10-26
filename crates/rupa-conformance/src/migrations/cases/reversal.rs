use rupa::migrate::{
    Blocker, Direction, Error, Plan, Planned, Policy, Recovery, RecoveryAction, RevertTarget,
    RunError,
};

use super::support::{
    ALLOW_LATE, blocked, history, planned, script, supports, transactional, undoable, version,
};
use super::{Apply, Harness, Inspect, Recover, Revert, catalog, versions};
use crate::migrations::fixture::{Body, CreateNotes, Undeclared, UndonePlans, reversible};

/// `100`, `101` and `102`, each creating a table its down body drops.
fn stack<H: Harness>() -> [Body; 3] {
    [
        undoable::<H>("100", "CREATE TABLE t100 (id INTEGER);", "DROP TABLE t100;"),
        undoable::<H>("101", "CREATE TABLE t101 (id INTEGER);", "DROP TABLE t101;"),
        undoable::<H>("102", "CREATE TABLE t102 (id INTEGER);", "DROP TABLE t102;"),
    ]
}

/// Installs the stack in the order `100, 102, 101`: `101` late, by permission.
fn install_out_of_order<H: Harness>(h: &H, entries: &[Body; 3]) {
    wait!(h, catalog(&[entries[0], entries[2]]).apply(h.connect())).expect("apply 100 and 102");
    wait!(h, catalog(entries).apply_with(h.connect(), &ALLOW_LATE)).expect("apply 101 late");
}

pub(super) fn case_revert_to_a_version_undoes_the_newest_installations_first<H: Harness>(
    h: &mut H,
) {
    let entries = stack::<H>();
    let stack = catalog(&entries);
    let target = RevertTarget::To(version("100"));

    install_out_of_order(h, &entries);

    let plan = wait!(h, stack.revert_plan(h.connect(), target)).expect("revert plan");
    let report = wait!(h, stack.revert(h.connect(), target)).expect("revert to 100");

    assert!(plan.status.blockers.is_empty(), "{plan:?}");
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.entry.version)
            .collect::<Vec<_>>(),
        report.reverted,
        "the plan lists what the revert runs"
    );
    assert_eq!(report.reverted, [version("101"), version("102")]);
    assert_eq!(report.remaining, [version("100")]);
    assert_eq!(history(h), ["100"]);
    assert!(h.table_exists("t100"));
    assert!(!h.table_exists("t101") && !h.table_exists("t102"));
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migrations WHERE kind = 'down' AND status = 'applied'"),
        2,
        "each undo appends its own event"
    );
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'applied'"),
        3,
        "undone installations keep their rows"
    );
}

pub(super) fn case_a_revert_that_is_not_a_suffix_is_refused_before_any_down<H: Harness>(h: &mut H) {
    let entries = stack::<H>();
    let stack = catalog(&entries);

    install_out_of_order(h, &entries);

    assert_eq!(
        blocked(wait!(
            h,
            stack.revert(h.connect(), RevertTarget::To(version("101")))
        )),
        [Blocker::Interleaved(version("101"))],
        "undoing only 102 would leave 101 installed on top of it"
    );
    assert_eq!(
        blocked(wait!(h, stack.revert(h.connect(), RevertTarget::Last(4)))),
        [Blocker::Exceeds {
            requested: 4,
            installed: 3,
        }]
    );
    assert_eq!(history(h), ["100", "102", "101"]);
    assert!(h.table_exists("t101") && h.table_exists("t102"));
}

pub(super) fn case_undoing_the_last_installation_makes_it_late_again<H: Harness>(h: &mut H) {
    let entries = stack::<H>();
    let stack = catalog(&entries);

    install_out_of_order(h, &entries);

    let report = wait!(h, stack.revert(h.connect(), RevertTarget::Last(1))).expect("undo 101");

    assert_eq!(report.reverted, [version("101")]);
    assert_eq!(report.remaining, [version("100"), version("102")]);
    assert_eq!(history(h), ["100", "102"]);
    assert!(
        matches!(
            blocked(wait!(h, stack.apply(h.connect()))).as_slice(),
            [Blocker::Late { version: late, .. }] if *late == version("101")
        ),
        "a strict apply refuses the undone version as late"
    );

    let reapplied = wait!(h, stack.apply_with(h.connect(), &ALLOW_LATE)).expect("reapply 101 late");

    assert_eq!(versions(&reapplied), ["101"]);
    assert_eq!(
        history(h),
        ["100", "102", "101"],
        "reapplying adds a new rank"
    );
}

pub(super) fn case_an_irreversible_installation_refuses_the_whole_revert<H: Harness>(h: &mut H) {
    let entries = [
        undoable::<H>("1", "CREATE TABLE t1 (id INTEGER);", "DROP TABLE t1;"),
        script::<H>("2", "CREATE TABLE t2 (id INTEGER);"),
        undoable::<H>("3", "CREATE TABLE t3 (id INTEGER);", "DROP TABLE t3;"),
    ];
    let mixed = catalog(&entries);

    wait!(h, mixed.apply(h.connect())).expect("apply");

    assert_eq!(
        blocked(wait!(h, mixed.revert(h.connect(), RevertTarget::Last(3)))),
        [Blocker::Irreversible(version("2"))]
    );
    assert_eq!(history(h), ["1", "2", "3"]);
    assert!(h.table_exists("t3"), "no down ran before the refusal");
}

pub(super) fn case_a_failed_down_keeps_earlier_undos_and_reports_the_rest<H: Harness>(h: &mut H) {
    let entries = [
        undoable::<H>("1", "CREATE TABLE t1 (id INTEGER);", "DROP TABLE t1;"),
        undoable::<H>("2", "CREATE TABLE t2 (id INTEGER);", "DROP TABLE absent;"),
        undoable::<H>("3", "CREATE TABLE t3 (id INTEGER);", "DROP TABLE t3;"),
    ];
    let failing = catalog(&entries);

    wait!(h, failing.apply(h.connect())).expect("apply");

    let error = wait!(h, failing.revert(h.connect(), RevertTarget::Last(3)))
        .expect_err("the down body of 2 fails");

    assert!(
        matches!(&error, RunError::Migration { version: failed, .. } if *failed == version("2")),
        "{error}"
    );
    assert_eq!(error.report().reverted, [version("3")]);
    assert_eq!(
        error.report().remaining,
        [version("1"), version("2")],
        "the error reports the active stack the failure left"
    );
    assert_eq!(history(h), ["1", "2"]);
    assert!(!h.table_exists("t3"), "the earlier undo stays committed");
    assert!(h.table_exists("t1") && h.table_exists("t2"));
}

pub(super) fn case_planned_and_sql_down_bodies_run_as_checksummed<H: Harness>(h: &mut H) {
    let plans = [Planned(UndonePlans(
        planned::<H>("1"),
        || Plan::new().sql("CREATE TABLE planned_notes (id INTEGER)"),
        || Plan::new().sql("DROP TABLE planned_notes"),
    ))];
    let plans = catalog(&plans);

    wait!(h, plans.apply(h.connect())).expect("apply the plan");

    let report = wait!(h, plans.revert(h.connect(), RevertTarget::Last(1))).expect("undo the plan");

    assert_eq!(report.reverted, [version("1")]);
    assert!(!h.table_exists("planned_notes"));

    if !transactional::<H>() {
        return;
    }

    let sql = [Planned(CreateNotes)];
    let sql = catalog(&sql);

    h.reset();
    wait!(h, sql.apply(h.connect())).expect("apply the SQL file");
    wait!(h, sql.revert(h.connect(), RevertTarget::Last(1))).expect("run its down file");

    assert!(!h.table_exists("notes"));
    assert!(history(h).is_empty());
}

pub(super) fn case_a_declared_down_without_a_body_rolls_back<H: Harness>(h: &mut H) {
    let entries = [Undeclared(script::<H>(
        "1",
        "CREATE TABLE kept (id INTEGER);",
    ))];
    let undeclared = catalog(&entries);
    let missing = |result: &Result<_, RunError<_, _, _>>| {
        matches!(
            result,
            Err(RunError::Infrastructure {
                source: Error::MissingDown(missing),
                ..
            }) if *missing == version("1")
        )
    };

    wait!(h, undeclared.apply(h.connect())).expect("apply");

    let first = wait!(h, undeclared.revert(h.connect(), RevertTarget::Last(1)));
    let second = wait!(h, undeclared.revert(h.connect(), RevertTarget::Last(1)));

    assert!(missing(&first), "{first:?}");
    assert!(
        missing(&second),
        "the failed attempt ran nothing, so it leaves nothing to recover: {second:?}"
    );
    assert_eq!(history(h), ["1"]);
    assert!(h.table_exists("kept"));
}

pub(super) fn case_an_uncertain_down_is_recovered_like_an_up<H: Harness>(h: &mut H) {
    if !supports::<H>(Policy::NonTransactional) {
        return;
    }

    let entries = [reversible(
        "1",
        "CREATE TABLE kept (id INTEGER);",
        "DROP TABLE kept; INSERT INTO absent VALUES (1);",
        Policy::NonTransactional,
    )];
    let partial = catalog(&entries);

    wait!(h, partial.apply(h.connect())).expect("apply");
    wait!(h, partial.revert(h.connect(), RevertTarget::Last(1))).expect_err("the down body fails");

    let status = wait!(h, partial.status(h.connect())).expect("status");
    let [attempt] = status.attempts.as_slice() else {
        panic!("one unresolved attempt: {status:?}");
    };
    let undone = status.history[0].installation_rank;
    let completed = Recovery::new(RecoveryAction::Completed(attempt.id), "the drop ran");

    assert_eq!(attempt.direction, Direction::Down { undoes: undone });
    assert_eq!(
        history(h),
        ["1"],
        "an uncertain undo leaves its installation"
    );

    wait!(h, partial.recover(h.connect(), &completed)).expect("assert the undo complete");

    assert!(history(h).is_empty(), "a recovered undo makes it inactive");
    assert!(!h.table_exists("kept"));
}
