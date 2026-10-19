use super::*;
use rupa::drivers::sqlite::{MigrationStateError, SqliteError};
use rupa::migrate::RunError;

#[test]
fn failed_later_body_rolls_back_its_changes_and_preserves_prior_commits() {
    let entries = [
        body(
            "1",
            "CREATE TABLE kept (id INTEGER); INSERT INTO kept VALUES (1);",
        ),
        body(
            "2",
            "CREATE TABLE lost (id INTEGER); INSERT INTO absent VALUES (2);",
        ),
    ];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");

    let mut runner = memory_runner();

    let error = runner.apply(&catalog).expect_err("second body fails");

    assert!(matches!(error, RunError::Migration { .. }));
    assert_eq!(
        error
            .applied()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["1"],
        "the error reports versions committed before the failure"
    );
    assert_eq!(
        runner
            .history()
            .expect("history")
            .iter()
            .map(|h| h.version.as_str())
            .collect::<Vec<_>>(),
        ["1"]
    );

    let mut executor = runner;
    assert_eq!(
        executor
            .connection()
            .query_row("SELECT count(*) FROM kept", [], |r| r.get::<_, i64>(0))
            .expect("committed row"),
        1
    );
    assert_eq!(
        executor
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name = 'lost'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("schema"),
        0
    );
    assert_eq!(
        executor
            .connection()
            .query_row(
                "SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'failed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("failed attempt"),
        1
    );
}

#[test]
fn changed_applied_body_is_rejected_before_pending_work() {
    let original = [body("1", "CREATE TABLE kept (id INTEGER);")];
    let changed = [
        body("1", "CREATE TABLE kept (id TEXT);"),
        body("2", "CREATE TABLE forbidden (id INTEGER);"),
    ];

    let mut runner = memory_runner();
    runner
        .apply(
            &Catalog::new("primary", VersionScheme::Numbered, &original).expect("original catalog"),
        )
        .expect("initial apply");

    assert!(matches!(
        runner.apply(
            &Catalog::new("primary", VersionScheme::Numbered, &changed).expect("changed catalog")
        ),
        Err(RunError::Infrastructure {
            source: Error::History(_),
            ..
        })
    ));
    assert_eq!(runner.history().expect("history").len(), 1);
}

#[test]
fn missing_applied_source_and_late_arrival_are_both_rejected() {
    let original = [
        body("1", "CREATE TABLE kept (id INTEGER);"),
        body("3", "INSERT INTO kept VALUES (3);"),
    ];
    let missing = [original[1]];
    let late = [
        original[0],
        body("2", "INSERT INTO kept VALUES (2);"),
        original[1],
    ];

    let mut runner = memory_runner();
    runner
        .apply(
            &Catalog::new("primary", VersionScheme::Numbered, &original).expect("original catalog"),
        )
        .expect("initial apply");

    assert!(
        runner
            .apply(
                &Catalog::new("primary", VersionScheme::Numbered, &missing)
                    .expect("missing-source catalog")
            )
            .is_err()
    );
    assert!(
        runner
            .apply(&Catalog::new("primary", VersionScheme::Numbered, &late).expect("late catalog"))
            .is_err()
    );
    assert_eq!(runner.history().expect("unchanged history").len(), 2);
}

#[test]
fn unmanaged_objects_are_never_treated_as_a_fresh_database() {
    let entries = [body("1", "CREATE TABLE migration_body (id INTEGER);")];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");

    let mut executor = SqliteExecutor::open_in_memory().expect("SQLite connection");
    executor
        .connection()
        .execute_batch("CREATE TABLE sqlitex_owned (id INTEGER)")
        .expect("existing user schema");
    let mut runner = executor;

    assert!(matches!(
        runner.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Unmanaged,
            ..
        })
    ));

    let mut executor = runner;
    assert_eq!(executor.connection().query_row("SELECT count(*) FROM sqlite_schema WHERE name LIKE '_rupa_%' OR name = 'migration_body'", [], |r| r.get::<_, i64>(0)).expect("no migration writes"), 0);
}

#[test]
fn incompatible_reserved_metadata_is_rejected_without_replacement() {
    let entries = [body("1", "SELECT 1;")];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");

    let mut executor = SqliteExecutor::open_in_memory().expect("SQLite connection");
    executor.connection().execute_batch("CREATE TABLE _rupa_migration_meta (valuable TEXT); INSERT INTO _rupa_migration_meta VALUES ('preserve');").expect("conflicting object");
    let mut runner = executor;

    assert!(runner.apply(&catalog).is_err());

    let mut executor = runner;
    assert_eq!(
        executor
            .connection()
            .query_row("SELECT valuable FROM _rupa_migration_meta", [], |r| r
                .get::<_, String>(0))
            .expect("preserved object"),
        "preserve"
    );
}

#[test]
fn inspection_does_not_initialize_history() {
    let mut runner = memory_runner();

    assert!(runner.history().expect("empty history").is_empty());

    let mut executor = runner;
    assert_eq!(
        executor
            .connection()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .expect("pristine schema"),
        0
    );
}

#[test]
fn unsupported_policy_is_rejected_before_bootstrap() {
    let original = body("1", "CREATE TABLE forbidden (id INTEGER);");
    let entries = [Body(Box::leak(Box::new(Metadata {
        policy: Policy::NonTransactional,
        ..*original.0
    })))];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid metadata");

    let mut runner = memory_runner();

    assert!(matches!(
        runner.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Validation(_),
            ..
        })
    ));
    assert!(runner.history().expect("no installed work").is_empty());
}

#[test]
fn malformed_planned_sql_is_rejected_with_the_session_dialect_before_bootstrap() {
    use rupa::migrate::{Operation, Planned, PlannedMigration, ValidationError};

    struct Unterminated(&'static Metadata);

    impl MigrationInfo for Unterminated {
        fn info(&self) -> &'static Metadata {
            self.0
        }
    }

    impl PlannedMigration for Unterminated {
        const UP: &'static [Operation] = &[Operation::Sql("INSERT INTO t VALUES ('oops")];
    }

    let entries = [Planned(Unterminated(
        body("1", "INSERT INTO t VALUES ('oops").0,
    ))];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries)
        .expect("lexing is deferred to the session dialect");

    let mut runner = memory_runner();

    assert!(matches!(
        runner.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Validation(ValidationError::Sql(22)),
            ..
        })
    ));
    assert!(runner.history().expect("no installed work").is_empty());
}

#[test]
fn version_scheme_cannot_change_after_initialization() {
    let entries = [body("1", "CREATE TABLE kept (id INTEGER);")];
    let empty: [Body; 0] = [];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("numbered catalog");

    let mut runner = memory_runner();
    runner.apply(&catalog).expect("initial apply");

    assert!(
        runner
            .apply(
                &Catalog::new("primary", VersionScheme::UtcTimestamp, &empty)
                    .expect("empty UTC catalog")
            )
            .is_err()
    );
    assert!(
        runner
            .apply(&catalog)
            .expect("original scheme still valid")
            .applied
            .is_empty()
    );
}

#[test]
fn failed_commit_releases_sqlite_writer_lock_before_returning() {
    let file = DatabaseFile::new();
    let entries = [body(
        "1",
        "CREATE TABLE parents(id INTEGER PRIMARY KEY); CREATE TABLE children(parent_id INTEGER REFERENCES parents(id) DEFERRABLE INITIALLY DEFERRED); INSERT INTO children VALUES (1);",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut connection = file.session();
    connection
        .connection()
        .execute_batch("PRAGMA foreign_keys = ON")
        .expect("enforce deferred constraint");
    let mut independent = file.session();
    independent
        .connection()
        .busy_timeout(std::time::Duration::ZERO)
        .expect("fail fast on locked writer");

    assert!(
        connection.apply(&catalog).is_err(),
        "deferred constraint fails at commit"
    );
    assert!(
        connection.connection().is_autocommit(),
        "failed completion must attempt rollback before returning"
    );
    independent
        .connection()
        .execute_batch("BEGIN IMMEDIATE; ROLLBACK")
        .expect("writer lock must be released even while failed connection is alive");
    assert!(
        connection
            .history()
            .expect("no committed migration")
            .is_empty()
    );
    assert!(
        matches!(
            rupa::core::exec::Executor::run(
                &mut connection,
                rupa::raw().sql("SELECT 1").execute::<u64>()
            ),
            Err(SqliteError::Migration(MigrationStateError::Poisoned))
        ),
        "a poisoned migration connection refuses statements with a typed state error"
    );
}

#[test]
fn public_apply_takes_a_native_connection_without_runner_wrappers() {
    let file = DatabaseFile::new();
    let entries = [body(
        "1",
        "CREATE TABLE once_only(id); INSERT INTO once_only VALUES (1);",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    assert_eq!(
        catalog
            .apply(file.session())
            .expect("apply owns the connection")
            .applied
            .len(),
        1
    );
    assert!(
        catalog
            .apply(file.session())
            .expect("fresh invocation inspects history")
            .applied
            .is_empty()
    );
}

/// A migration with an application-specific error distinct from the driver's error.
struct ApplicationFailure;
impl MigrationInfo for ApplicationFailure {
    fn info(&self) -> &'static Metadata {
        const INFO: Metadata = Metadata {
            version: "2",
            description: "application error",
            policy: Policy::TransactionalRequired,
            kind: MigrationKind::Incremental,
            inputs: &[ArtifactInput::new(
                "application.rs",
                include_str!("failures.rs"),
            )],
        };

        &INFO
    }
}
impl<S: Session> Migration<S> for ApplicationFailure {
    type Error = std::io::Error;

    fn up(&self, _: &mut Context<'_, S>) -> Result<(), Self::Error> {
        Err(std::io::Error::other("application rejected migration"))
    }
}

rupa::migrate::migration_catalog! {
    /// Heterogeneous static errors do not require boxing the migration body.
    enum ApplicationCatalog, error ApplicationCatalogError {
        Sql(rupa::migrate::Planned<application::CreateNotes>),
        Custom(ApplicationFailure),
    }
}

#[test]
fn mixed_catalog_preserves_application_specific_error_types() {
    let entries = [
        ApplicationCatalog::Sql(rupa::migrate::Planned(application::CreateNotes)),
        ApplicationCatalog::Custom(ApplicationFailure),
    ];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let error = catalog
        .apply(memory_runner())
        .expect_err("application body fails");

    match error {
        RunError::Migration {
            source: ApplicationCatalogError::Custom(source),
            ..
        } => assert_eq!(source.kind(), std::io::ErrorKind::Other),
        other => panic!("application error must retain its concrete type: {other}"),
    }
}
