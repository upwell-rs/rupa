use rupa::migrate::{Blocker, Error, Recovery, RecoveryAction, RunError};

use super::support::{ALLOW_LATE, blocked, busy, history, script, transactional, version};
use super::{Apply, Harness, Inspect, Recover, catalog, versions};
use crate::migrations::fixture::{Body, Interrupted};

/// `100` and `102`, which a target installs before `101` appears.
fn installed<H: Harness>() -> [Body; 2] {
    [
        script::<H>("100", "CREATE TABLE ordered (id INTEGER);"),
        script::<H>("102", "INSERT INTO ordered VALUES (102);"),
    ]
}

/// The installed pair with `101` between them.
fn with_late<H: Harness>() -> [Body; 3] {
    let [first, last] = installed::<H>();

    [
        first,
        script::<H>("101", "INSERT INTO ordered VALUES (101);"),
        last,
    ]
}

pub(super) fn case_pending_versions_run_in_version_order<H: Harness>(h: &mut H) {
    let entries = with_late::<H>();

    wait!(h, catalog(&entries[..1]).apply(h.connect())).expect("apply 100");

    let report = wait!(h, catalog(&entries).apply(h.connect())).expect("apply the rest");

    assert_eq!(versions(&report), ["101", "102"]);
    assert!(
        report.late.is_empty(),
        "nothing ran below an installed version"
    );
    assert_eq!(history(h), ["100", "101", "102"]);
}

pub(super) fn case_strict_apply_refuses_a_late_version_before_running_it<H: Harness>(h: &mut H) {
    let late = with_late::<H>();
    let late = catalog(&late);

    wait!(h, catalog(&installed::<H>()).apply(h.connect())).expect("apply 100 and 102");

    let plan = wait!(h, late.plan(h.connect())).expect("strict plan");

    assert!(
        matches!(
            blocked(wait!(h, late.apply(h.connect()))).as_slice(),
            [Blocker::Late { version: late, .. }] if *late == version("101")
        ),
        "strict apply reports the late version"
    );
    assert!(
        matches!(
            plan.status.blockers.as_slice(),
            [Blocker::Late { version: late, .. }] if *late == version("101")
        ),
        "a strict plan reports what apply refuses: {plan:?}"
    );
    assert!(plan.late.is_empty());
    assert_eq!(history(h), ["100", "102"]);
    assert_eq!(h.count("SELECT count(*) FROM ordered"), 1, "101 never ran");
}

pub(super) fn case_permitted_late_version_runs_last_with_a_new_rank<H: Harness>(h: &mut H) {
    let late = with_late::<H>();
    let late = catalog(&late);

    wait!(h, catalog(&installed::<H>()).apply(h.connect())).expect("apply 100 and 102");

    let plan = wait!(h, late.plan_with(h.connect(), &ALLOW_LATE)).expect("permitted plan");

    assert!(plan.status.blockers.is_empty(), "{plan:?}");
    assert_eq!(plan.late, [version("101")]);
    assert_eq!(
        plan.steps
            .iter()
            .map(|step| step.entry.version)
            .collect::<Vec<_>>(),
        [version("101")]
    );

    let report = wait!(h, late.apply_with(h.connect(), &ALLOW_LATE)).expect("permitted apply");

    assert_eq!(versions(&report), ["101"]);
    assert_eq!(report.late, [version("101")], "the report labels late work");
    assert_eq!(history(h), ["100", "102", "101"]);
    assert_eq!(h.count("SELECT count(*) FROM ordered"), 2);
    assert!(
        wait!(h, late.apply(h.connect()))
            .expect("a strict apply afterwards")
            .applied
            .is_empty(),
        "an installed late version is no longer late"
    );
}

pub(super) fn case_a_failed_permitted_run_still_reports_its_late_commits<H: Harness>(h: &mut H) {
    let [first, late, last] = with_late::<H>();
    let failing = [
        first,
        late,
        last,
        script::<H>("103", "INSERT INTO absent VALUES (103);"),
    ];

    wait!(h, catalog(&[first, last]).apply(h.connect())).expect("apply 100 and 102");

    let error =
        wait!(h, catalog(&failing).apply_with(h.connect(), &ALLOW_LATE)).expect_err("103 fails");

    assert_eq!(error.applied(), [version("101")]);
    assert_eq!(
        error.late(),
        [version("101")],
        "the error keeps the late label of what committed"
    );
}

pub(super) fn case_late_permission_never_lifts_other_blockers<H: Harness>(h: &mut H) {
    let [first, late, last] = with_late::<H>();
    let edited = [
        script::<H>("100", "CREATE TABLE edited (id INTEGER);"),
        late,
        last,
    ];
    let missing = [late, last];

    wait!(h, catalog(&[first, last]).apply(h.connect())).expect("apply 100 and 102");

    assert!(
        matches!(
            blocked(wait!(h, catalog(&edited).apply_with(h.connect(), &ALLOW_LATE))).as_slice(),
            [Blocker::Changed { version: changed, .. }] if *changed == version("100")
        ),
        "a changed artifact still refuses the run"
    );
    assert!(
        matches!(
            blocked(wait!(h, catalog(&missing).apply_with(h.connect(), &ALLOW_LATE))).as_slice(),
            [Blocker::Missing { version: missing, .. }] if missing == "100"
        ),
        "a missing artifact still refuses the run"
    );
    assert_eq!(history(h), ["100", "102"]);
    assert_eq!(h.count("SELECT count(*) FROM ordered"), 1, "101 never ran");

    if !transactional::<H>() {
        return;
    }

    let corrected = [script::<H>("1", "CREATE TABLE corrected (id INTEGER);")];
    let corrected = catalog(&corrected);
    let release = Recovery::new(RecoveryAction::ReleaseClaim, "the runner process exited");

    h.reset();
    interrupt!(h, catalog(&[Interrupted]));

    let mut report = wait!(h, corrected.apply_with(h.connect(), &ALLOW_LATE));

    if busy(&report) {
        wait!(h, corrected.recover(h.connect(), &release)).expect("the stale claim is released");
        report = wait!(h, corrected.apply_with(h.connect(), &ALLOW_LATE));
    }

    assert!(
        matches!(
            report,
            Err(RunError::Infrastructure {
                source: Error::Reconciled(_),
                ..
            })
        ),
        "an interrupted attempt still stops the run"
    );
    assert!(!h.table_exists("corrected"));
}
