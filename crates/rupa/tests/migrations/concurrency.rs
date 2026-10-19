use super::*;
use rupa::migrate::RunError;

#[test]
fn durable_claim_excludes_another_connection_between_transactions() {
    let file = DatabaseFile::new();
    let entries = [body("1", "CREATE TABLE once_only(id INTEGER);")];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut owner = file.session();
    let mut competitor = file.session();
    owner
        .acquire("primary", VersionScheme::Numbered)
        .expect("first bootstrap and claim");

    assert!(matches!(
        competitor.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Busy,
            ..
        })
    ));
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

#[test]
fn concurrent_first_runners_execute_each_artifact_once() {
    let file = DatabaseFile::new();
    let entries = [body(
        "1",
        "CREATE TABLE marker(id INTEGER); INSERT INTO marker VALUES (1);",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let barrier = std::sync::Barrier::new(2);
    let first = file.session();
    let second = file.session();
    let reports = std::thread::scope(|scope| {
        let launch = |session: SqliteExecutor| {
            scope.spawn(|| {
                let mut runner = session;
                barrier.wait();

                runner.apply(&catalog)
            })
        };
        let one = launch(first);
        let two = launch(second);

        [
            one.join().expect("first runner thread"),
            two.join().expect("second runner thread"),
        ]
    });

    let mut applied = 0;
    for report in reports {
        match report {
            Ok(report) => applied += report.applied.len(),
            Err(RunError::Infrastructure {
                source: Error::Busy,
                ..
            }) => {}
            Err(other) => panic!("unexpected concurrency failure: {other}"),
        }
    }

    assert_eq!(applied, 1);
    let mut executor = file.session();
    assert_eq!(
        executor
            .connection()
            .query_row("SELECT count(*) FROM marker", [], |r| r.get::<_, i64>(0))
            .expect("one execution"),
        1
    );
}

/// Panics after transactional DDL to exercise rollback on session destruction.
struct Interrupted;
impl MigrationInfo for Interrupted {
    fn info(&self) -> &'static Metadata {
        const INFO: Metadata = Metadata {
            version: "1",
            description: "interruption",
            policy: Policy::TransactionalRequired,
            kind: MigrationKind::Incremental,
            inputs: &[ArtifactInput::new(
                "interruption.rs",
                include_str!("concurrency.rs"),
            )],
        };

        &INFO
    }
}
impl<S: Session> Migration<S> for Interrupted {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("CREATE TABLE lost(id INTEGER)")?;
        panic!("injected migration interruption");
    }
}

#[test]
fn interrupted_body_rolls_back_but_retains_claim_and_unresolved_attempt() {
    let file = DatabaseFile::new();
    let entries = [Interrupted];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut runner = file.session();
        let _ = runner.apply(&catalog);
    }));
    let mut competitor = file.session();

    assert!(interrupted.is_err());
    assert!(matches!(
        competitor.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Busy,
            ..
        })
    ));

    let mut executor = file.session();
    assert_eq!(
        executor
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'lost'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("rolled-back schema"),
        0
    );
    assert_eq!(
        executor
            .connection()
            .query_row(
                "SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("unresolved attempt"),
        1
    );
}
