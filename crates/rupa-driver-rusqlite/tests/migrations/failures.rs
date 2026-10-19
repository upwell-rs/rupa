use rupa::migrate::sync::Session;
use rupa::migrate::{Catalog, Error, RunError, StateError, VersionScheme};
use rupa_conformance::migrations::fixture::body;
use rupa_driver_rusqlite::SqliteError;

use super::DatabaseFile;

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
            Err(SqliteError::Migration(StateError::Poisoned))
        ),
        "a poisoned migration connection refuses statements with a typed state error"
    );
}

#[test]
fn names_merely_resembling_sqlite_internals_make_a_target_unmanaged() {
    let file = DatabaseFile::new();
    let entries = [body("1", "CREATE TABLE migration_body (id INTEGER);")];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    file.connection()
        .execute_batch("CREATE TABLE sqlitex_owned (id INTEGER)")
        .expect("user table with a sqlite-like prefix");

    assert!(matches!(
        file.session().apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Unmanaged,
            ..
        })
    ));
}
