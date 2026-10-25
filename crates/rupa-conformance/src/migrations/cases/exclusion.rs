use rupa::migrate::VersionScheme;

use super::support::{busy, script};
use super::{Apply, Harness, Session, catalog};

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

pub(super) fn case_concurrent_first_runners_execute_once<H: Harness + Sync>(h: &mut H) {
    let entries = [script::<H>(
        "1",
        "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
    )];
    let catalog = catalog(&entries);
    let barrier = std::sync::Barrier::new(2);
    let mut applied = 0;

    let target = &*h;
    let sessions = [target.connect(), target.connect()];
    let reports = std::thread::scope(|scope| {
        sessions
            .map(|mut session| {
                let (barrier, catalog) = (&barrier, &catalog);

                scope.spawn(move || {
                    barrier.wait();

                    wait!(target, session.apply(catalog))
                })
            })
            .map(|runner| runner.join().expect("runner thread"))
    });

    for report in reports {
        if busy(&report) {
            continue;
        }

        applied += report
            .expect("a runner applies or finds the target busy")
            .applied
            .len();
    }

    assert_eq!(applied, 1);
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
