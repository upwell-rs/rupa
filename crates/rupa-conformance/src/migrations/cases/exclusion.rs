use rupa::migrate::{ApplyReport, Catalog, VersionScheme};

use super::support::{busy, script, snapshot_of};
use super::{Apply, Harness, Session, catalog};
use crate::migrations::fixture::Body;

pub(super) fn case_owner_excludes_competitors_between_commits<H: Harness>(h: &mut H) {
    let entries = [script::<H>("1", "CREATE TABLE once_only(id INTEGER);")];
    let catalog = catalog(&entries);

    let mut owner = h.connect();
    let mut competitor = h.connect();
    wait!(h, owner.acquire("primary", VersionScheme::Numbered)).expect("first bootstrap and claim");

    assert!(busy(&wait!(h, competitor.apply(&catalog))));

    wait!(h, owner.release()).expect("release claim");

    assert_eq!(
        wait!(h, competitor.apply(&catalog))
            .expect("apply after release")
            .applied
            .len(),
        1
    );
}

/// Applies `catalog` from two runners released together, returning the
/// report of each runner that did not find the target busy.
fn race<H: Harness + Sync>(h: &H, catalog: &Catalog<'_, Body>) -> Vec<ApplyReport> {
    let barrier = std::sync::Barrier::new(2);
    let sessions = [h.connect(), h.connect()];
    let reports = std::thread::scope(|scope| {
        sessions
            .map(|mut session| {
                let barrier = &barrier;

                scope.spawn(move || {
                    barrier.wait();

                    wait!(h, session.apply(catalog))
                })
            })
            .map(|runner| runner.join().expect("runner thread"))
    });

    reports
        .into_iter()
        .filter(|report| !busy(report))
        .map(|report| report.expect("a runner applies or finds the target busy"))
        .collect()
}

pub(super) fn case_concurrent_first_runners_execute_once<H: Harness + Sync>(h: &mut H) {
    let entries = [script::<H>(
        "1",
        "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
    )];

    let reports = race(h, &catalog(&entries));

    assert_eq!(
        reports
            .iter()
            .map(|report| report.applied.len())
            .sum::<usize>(),
        1
    );
    assert_eq!(h.count("SELECT count(*) FROM marker"), 1);
}

pub(super) fn case_concurrent_fresh_runners_run_a_snapshot_once<H: Harness + Sync>(h: &mut H) {
    let entries = [
        script::<H>(
            "1",
            "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
        ),
        snapshot_of::<H>(
            "1",
            "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
        ),
    ];

    let reports = race(h, &catalog(&entries));

    assert_eq!(
        reports
            .iter()
            .filter(|report| report.snapshot.is_some())
            .count(),
        1,
        "the second runner reclassifies the target under exclusion"
    );
    assert!(reports.iter().all(|report| report.applied.is_empty()));
    assert_eq!(h.count("SELECT count(*) FROM marker"), 1);
}

pub(super) fn case_apply_takes_and_closes_a_native_connection<H: Harness>(h: &mut H) {
    let entries = [script::<H>(
        "1",
        "CREATE TABLE once_only(id INTEGER); INSERT INTO once_only VALUES (1);",
    )];
    let catalog = catalog(&entries);

    assert_eq!(
        wait!(h, catalog.apply(h.connect()))
            .expect("apply owns the connection")
            .applied
            .len(),
        1
    );
    assert!(
        wait!(h, catalog.apply(h.connect()))
            .expect("a fresh invocation inspects history")
            .applied
            .is_empty()
    );
    assert_eq!(h.count("SELECT count(*) FROM once_only"), 1);
}
