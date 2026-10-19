use rupa::migrate::{
    Catalog, Error, Plan, Planned, Policy, RunError, ScriptError, Session, ValidationError,
    VersionScheme,
};

use super::fixture::{
    ApplicationFailure, Body, CreateNotes, Failing, FailingError, Mixed, Normalize, Plans, body,
    planned, with_policy,
};
use super::{MigrationHarness, catalog, versions};

pub(super) fn case_mixed_apply_records_history_and_never_replays<H: MigrationHarness>(h: &mut H) {
    let entries = [Mixed::Rust(Normalize), Mixed::Schema(Planned(CreateNotes))];
    let catalog = catalog(&entries);

    let mut session = h.connect();
    let report = session.apply(&catalog).expect("mixed apply");

    assert_eq!(versions(&report), ["1", "2"]);
    assert!(
        session
            .apply(&catalog)
            .expect("idempotent apply")
            .applied
            .is_empty()
    );
    assert_eq!(
        session
            .history()
            .expect("installed history")
            .iter()
            .map(|entry| entry.installation_rank)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        h.count("SELECT count(*) FROM notes WHERE title = 'HELLO; WORLD'"),
        1
    );
}

pub(super) fn case_failed_body_rolls_back_and_preserves_prior_commits<H: MigrationHarness>(
    h: &mut H,
) {
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

    let mut session = h.connect();
    let error = session
        .apply(&catalog(&entries))
        .expect_err("second body fails");

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
        session
            .history()
            .expect("history")
            .iter()
            .map(|entry| entry.version.as_str())
            .collect::<Vec<_>>(),
        ["1"]
    );
    assert_eq!(h.count("SELECT count(*) FROM kept"), 1);
    assert!(!h.table_exists("lost"));
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'failed'"),
        1
    );
}

pub(super) fn case_changed_applied_body_is_rejected_before_pending_work<H: MigrationHarness>(
    h: &mut H,
) {
    let original = [body("1", "CREATE TABLE kept (id INTEGER);")];
    let changed = [
        body("1", "CREATE TABLE kept (id TEXT);"),
        body("2", "CREATE TABLE forbidden (id INTEGER);"),
    ];

    let mut session = h.connect();
    session.apply(&catalog(&original)).expect("initial apply");

    assert!(matches!(
        session.apply(&catalog(&changed)),
        Err(RunError::Infrastructure {
            source: Error::History(_),
            ..
        })
    ));
    assert_eq!(session.history().expect("history").len(), 1);
    assert!(!h.table_exists("forbidden"));
}

pub(super) fn case_missing_source_and_late_arrival_are_rejected<H: MigrationHarness>(h: &mut H) {
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

    let mut session = h.connect();
    session.apply(&catalog(&original)).expect("initial apply");

    assert!(session.apply(&catalog(&missing)).is_err());
    assert!(session.apply(&catalog(&late)).is_err());
    assert_eq!(session.history().expect("unchanged history").len(), 2);
    assert_eq!(h.count("SELECT count(*) FROM kept"), 1);
}

pub(super) fn case_unmanaged_objects_are_never_a_fresh_target<H: MigrationHarness>(h: &mut H) {
    let entries = [body("1", "CREATE TABLE migration_body (id INTEGER);")];

    h.execute("CREATE TABLE unmanaged_owned (id INTEGER)");

    assert!(matches!(
        h.connect().apply(&catalog(&entries)),
        Err(RunError::Infrastructure {
            source: Error::Unmanaged,
            ..
        })
    ));
    assert!(!h.table_exists("_rupa_migration_meta"));
    assert!(!h.table_exists("migration_body"));
}

pub(super) fn case_incompatible_reserved_metadata_is_preserved<H: MigrationHarness>(h: &mut H) {
    let entries = [body("1", "SELECT 1;")];

    h.execute(
        "CREATE TABLE _rupa_migration_meta (valuable TEXT); INSERT INTO _rupa_migration_meta VALUES ('preserve');",
    );

    assert!(h.connect().apply(&catalog(&entries)).is_err());
    assert_eq!(
        h.count("SELECT count(*) FROM _rupa_migration_meta WHERE valuable = 'preserve'"),
        1
    );
}

pub(super) fn case_inspection_does_not_initialize_history<H: MigrationHarness>(h: &mut H) {
    assert!(h.connect().history().expect("empty history").is_empty());

    for table in [
        "_rupa_migration_meta",
        "_rupa_migration_history",
        "_rupa_migration_attempts",
    ] {
        assert!(!h.table_exists(table), "{table}");
    }
}

pub(super) fn case_version_scheme_cannot_change<H: MigrationHarness>(h: &mut H) {
    let entries = [body("1", "CREATE TABLE kept (id INTEGER);")];
    let empty: [Body; 0] = [];
    let numbered = catalog(&entries);
    let utc = Catalog::new("primary", VersionScheme::UtcTimestamp, &empty).expect("UTC catalog");

    let mut session = h.connect();
    session.apply(&numbered).expect("initial apply");

    assert!(session.apply(&utc).is_err());
    assert!(
        session
            .apply(&numbered)
            .expect("original scheme still valid")
            .applied
            .is_empty()
    );
}

pub(super) fn case_unsupported_policy_is_rejected_before_bootstrap<H: MigrationHarness>(h: &mut H) {
    let entries = [with_policy(
        "1",
        "CREATE TABLE forbidden (id INTEGER);",
        Policy::NonTransactional,
    )];

    assert!(matches!(
        h.connect().apply(&catalog(&entries)),
        Err(RunError::Infrastructure {
            source: Error::Validation(ValidationError::Unsupported(_)),
            ..
        })
    ));
    assert!(!h.table_exists("_rupa_migration_meta"));
}

pub(super) fn case_malformed_planned_sql_is_rejected_before_bootstrap<H: MigrationHarness>(
    h: &mut H,
) {
    let entries = [Planned(Plans(planned("1"), || {
        Plan::new().sql("INSERT INTO t VALUES ('oops")
    }))];

    assert!(matches!(
        h.connect().apply(&catalog(&entries)),
        Err(RunError::Infrastructure {
            source: Error::Validation(ValidationError::Script(ScriptError::Unterminated(22))),
            ..
        })
    ));
    assert!(!h.table_exists("_rupa_migration_meta"));
}

pub(super) fn case_application_errors_keep_their_types<H: MigrationHarness>(h: &mut H) {
    let entries = [
        Failing::Sql(Planned(CreateNotes)),
        Failing::Custom(ApplicationFailure),
    ];

    match catalog(&entries).apply(h.connect()) {
        Err(RunError::Migration {
            source: FailingError::Custom(source),
            ..
        }) => assert_eq!(source.kind(), std::io::ErrorKind::Other),
        Err(other) => panic!("application error must retain its concrete type: {other}"),
        Ok(report) => panic!("application body must fail, applied {:?}", report.applied),
    }
}
