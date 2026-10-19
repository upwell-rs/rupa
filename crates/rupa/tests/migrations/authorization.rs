use super::*;

#[test]
fn scripts_cannot_commit_release_savepoints_attach_or_edit_history() {
    let scripts = [
        "CREATE TABLE marker(id); COMMIT;",
        "CREATE TABLE marker(id); ROLLBACK;",
        "CREATE TABLE marker(id); SAVEPOINT escape;",
        "CREATE TABLE marker(id); ATTACH ':memory:' AS other;",
        "CREATE TABLE marker(id); PRAGMA writable_schema = ON;",
        "CREATE TABLE marker(id); UPDATE _rupa_migration_meta SET owner = NULL;",
        "CREATE TABLE marker(id); DELETE FROM _RUPA_MIGRATION_ATTEMPTS;",
        "CREATE TABLE marker(id); CREATE TRIGGER hijack AFTER INSERT ON _rupa_migration_history BEGIN DELETE FROM marker; END;",
        "CREATE TABLE marker(id); SELECT * FROM _rupa_migration_history;",
    ];

    for sql in scripts {
        let entries = [body("1", sql)];
        let catalog =
            Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid source");
        let mut runner = memory_runner();

        assert!(runner.apply(&catalog).is_err(), "{sql}");
        assert!(runner.history().expect("no committed body").is_empty());

        let mut executor = runner;
        assert_eq!(
            executor
                .connection()
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name = 'marker'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .expect("rolled-back schema"),
            0,
            "{sql}"
        );
    }
}

/// Uses the typed-query surface to attempt transaction control.
struct RawCommit;
impl MigrationInfo for RawCommit {
    fn info(&self) -> &'static Metadata {
        const INFO: Metadata = Metadata {
            version: "1",
            description: "raw control",
            policy: Policy::TransactionalRequired,
            kind: MigrationKind::Incremental,
            inputs: &[ArtifactInput::new(
                "raw.rs",
                include_str!("authorization.rs"),
            )],
        };

        &INFO
    }
}
impl<S: Session> Migration<S> for RawCommit {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script("CREATE TABLE marker(id)")?;
        context.run(rupa::raw().sql("COMMIT").execute::<u64>())?;

        Ok(())
    }
}

#[test]
fn raw_typed_queries_cannot_escape_the_runner_transaction() {
    let entries = [RawCommit];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid source");

    let mut runner = memory_runner();

    assert!(runner.apply(&catalog).is_err());
    assert!(runner.history().expect("empty history").is_empty());
}

#[test]
fn ordinary_table_and_column_renames_work_with_native_sqlite_parsing() {
    let entries = [body(
        "1",
        "CREATE TABLE notes(old_name TEXT); ALTER TABLE notes RENAME COLUMN old_name TO title; ALTER TABLE notes RENAME TO renamed; INSERT INTO renamed VALUES ('ok');",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut connection = memory_runner();

    connection
        .apply(&catalog)
        .expect("ordinary DDL must remain supported");
    assert_eq!(
        connection
            .connection()
            .query_row("SELECT title FROM renamed", [], |r| r.get::<_, String>(0))
            .expect("renamed row"),
        "ok"
    );
}

#[test]
fn renaming_a_user_table_into_reserved_metadata_cannot_commit() {
    let entries = [body(
        "1",
        "CREATE TABLE notes(id); ALTER TABLE notes RENAME TO _rupa_hidden;",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut connection = memory_runner();

    assert!(connection.apply(&catalog).is_err());
    assert_eq!(
        connection
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('notes', '_rupa_hidden')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .expect("rollback"),
        0
    );
}

#[test]
fn user_sql_cannot_corrupt_framework_autoincrement_sequences() {
    let corrupt = [body(
        "1",
        "UPDATE sqlite_sequence SET seq = 9223372036854775807 WHERE name = '_rupa_migration_attempts';",
    )];
    let valid = [body(
        "1",
        "CREATE TABLE user_rows(id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO user_rows DEFAULT VALUES;",
    )];

    let mut connection = memory_runner();

    assert!(
        connection
            .apply(&Catalog::new("primary", VersionScheme::Numbered, &corrupt).expect("catalog"))
            .is_err()
    );
    connection
        .apply(&Catalog::new("primary", VersionScheme::Numbered, &valid).expect("catalog"))
        .expect("internal sequences and user AUTOINCREMENT remain usable");
    assert_eq!(
        connection
            .connection()
            .query_row("SELECT id FROM user_rows", [], |r| r.get::<_, i64>(0))
            .expect("generated ID"),
        1
    );
}

#[test]
fn internal_sequence_names_cannot_be_reassigned_and_user_autoincrement_tables_can_be_dropped() {
    let corrupt = [body(
        "1",
        "UPDATE sqlite_sequence SET name = 'stolen' WHERE name = '_rupa_migration_attempts';",
    )];
    let valid = [body(
        "1",
        "CREATE TABLE disposable(id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO disposable DEFAULT VALUES; DROP TABLE disposable;",
    )];

    let mut connection = memory_runner();

    assert!(
        connection
            .apply(&Catalog::new("primary", VersionScheme::Numbered, &corrupt).expect("catalog"))
            .is_err()
    );
    connection
        .apply(&Catalog::new("primary", VersionScheme::Numbered, &valid).expect("catalog"))
        .expect("ordinary AUTOINCREMENT DDL");
}

#[test]
fn migration_support_does_not_make_native_transaction_methods_ambiguous() {
    use rupa::{Transaction, Transactional};

    let mut connection = memory_runner();

    let transaction = connection.begin().expect("ordinary native transaction");
    transaction.rollback().expect("ordinary native rollback");
}
