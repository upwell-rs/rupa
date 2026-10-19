use rupa::migrate::{Error, RunError, Session, VersionScheme};

use super::fixture::{Interrupted, body};
use super::{MigrationHarness, catalog};

fn busy<E: std::error::Error, M: std::error::Error>(
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

pub(super) fn case_owner_excludes_competitors_between_commits<H: MigrationHarness>(h: &mut H) {
    let entries = [body("1", "CREATE TABLE once_only(id INTEGER);")];
    let catalog = catalog(&entries);

    let mut owner = h.connect();
    let mut competitor = h.connect();
    owner
        .acquire("primary", VersionScheme::Numbered)
        .expect("first bootstrap and claim");

    assert!(busy(&competitor.apply(&catalog)));

    owner.release().expect("release claim");

    assert_eq!(
        competitor
            .apply(&catalog)
            .expect("apply after release")
            .applied
            .len(),
        1
    );
}

pub(super) fn case_concurrent_first_runners_execute_once<H: MigrationHarness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
    )];
    let catalog = catalog(&entries);
    let barrier = std::sync::Barrier::new(2);
    let mut applied = 0;

    let sessions = [h.connect(), h.connect()];
    let reports = std::thread::scope(|scope| {
        sessions
            .map(|mut session| {
                let (barrier, catalog) = (&barrier, &catalog);

                scope.spawn(move || {
                    barrier.wait();

                    session.apply(catalog)
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

pub(super) fn case_interrupted_body_leaves_an_unresolved_attempt<H: MigrationHarness>(h: &mut H) {
    let entries = [Interrupted];
    let catalog = catalog(&entries);

    let session = h.connect();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = catalog.apply(session);
    }));
    let competitor = h.connect().apply(&catalog);

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
    assert!(!h.table_exists("lost"));
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'"),
        1
    );
}

pub(super) fn case_apply_takes_and_closes_a_native_connection<H: MigrationHarness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE once_only(id INTEGER); INSERT INTO once_only VALUES (1);",
    )];
    let catalog = catalog(&entries);

    assert_eq!(
        catalog
            .apply(h.connect())
            .expect("apply owns the connection")
            .applied
            .len(),
        1
    );
    assert!(
        catalog
            .apply(h.connect())
            .expect("a fresh invocation inspects history")
            .applied
            .is_empty()
    );
    assert_eq!(h.count("SELECT count(*) FROM once_only"), 1);
}
