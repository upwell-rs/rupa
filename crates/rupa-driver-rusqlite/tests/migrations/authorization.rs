use rupa::migrate::sync::Session;
use rupa::migrate::{Catalog, VersionScheme};
use rupa_conformance::migrations::fixture::body;

use super::memory_runner;

#[test]
fn ordinary_indexes_can_be_created_and_dropped() {
    let entries = [body(
        "1",
        "CREATE TABLE notes(title TEXT); INSERT INTO notes VALUES ('a'); CREATE UNIQUE INDEX notes_title ON notes(title); REINDEX notes_title; DROP INDEX notes_title;",
    )];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut connection = memory_runner();

    connection
        .apply(&catalog)
        .expect("SQLite authorizes CREATE INDEX as a reindex of the new index");
}

#[test]
fn user_sql_cannot_corrupt_framework_autoincrement_sequences() {
    let corrupt = [body(
        "1",
        "UPDATE sqlite_sequence SET seq = 9223372036854775807 WHERE name = '_rupa_migrations';",
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
        "UPDATE sqlite_sequence SET name = 'stolen' WHERE name = '_rupa_migrations';",
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
