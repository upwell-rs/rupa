use rupa::migrate::sync::Session;
use rupa::migrate::{Catalog, Error, RunError, StateError, VersionScheme};
use rupa_conformance::migrations::fixture::{Interrupted, body};
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
            rupa::raw()
                .sql("SELECT 1")
                .execute::<u64>()
                .execute(&connection),
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

#[test]
fn a_panicking_run_poisons_a_retained_session() {
    let file = DatabaseFile::new();
    let entries = [Interrupted];
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries).expect("catalog");

    let mut session = file.session();
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = session.apply(&catalog);
    }));

    assert!(interrupted.is_err());
    assert!(
        matches!(
            rupa::raw()
                .sql("SELECT 1")
                .execute::<u64>()
                .execute(&session),
            Err(SqliteError::Migration(StateError::Poisoned))
        ),
        "a retained session refuses use after a run unwound"
    );
}

#[test]
fn release_restores_the_connection_busy_timeout() {
    let target = DatabaseFile::new();
    let busy_timeout = |session: &mut rupa_driver_rusqlite::SqliteExecutor| -> i64 {
        session
            .connection()
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout")
    };

    let mut session = target.session();

    session
        .connection()
        .execute_batch("PRAGMA busy_timeout = 1234")
        .expect("application busy_timeout");
    session
        .acquire("primary", VersionScheme::Numbered)
        .expect("acquire");

    let during = busy_timeout(&mut session);

    session.release().expect("release");

    assert_eq!(during, 5000, "the runner waits its own lock timeout");
    assert_eq!(busy_timeout(&mut session), 1234);
}
