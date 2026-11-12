//! Runs the shared migration cases and SQLite-specific migration behaviour.

mod authorization;
mod failures;
mod schema;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use rupa_conformance::migrations::asynchronous::{self, Blocked};
use rupa_conformance::migrations::sync::{self, Harness, Target};
use rupa_driver_rusqlite::SqliteExecutor;

/// A disposable file target shared by independent SQLite connections.
struct DatabaseFile(PathBuf);

impl DatabaseFile {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        Self(std::env::temp_dir().join(format!(
            "rupa-migrations-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn session(&self) -> SqliteExecutor {
        SqliteExecutor::open(&self.0).expect("file connection")
    }

    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.0).expect("inspection connection")
    }
}

impl Drop for DatabaseFile {
    fn drop(&mut self) {
        if self.0.exists() {
            std::fs::remove_file(&self.0).expect("remove temporary database");
        }
    }
}

impl Harness for DatabaseFile {
    type Session = SqliteExecutor;

    fn connect(&self) -> SqliteExecutor {
        self.session()
    }
}

impl Target for DatabaseFile {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn execute(&self, sql: &str) {
        self.connection().execute_batch(sql).expect("setup SQL");
    }

    fn count(&self, sql: &str) -> i64 {
        self.connection()
            .query_row(sql, [], |row| row.get(0))
            .expect("inspection count")
    }

    fn table_exists(&self, name: &str) -> bool {
        self.connection()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [name],
                |row| row.get::<_, i64>(0),
            )
            .expect("schema inspection")
            != 0
    }

    fn forbidden_scripts(&self) -> &'static [&'static str] {
        &[
            "CREATE TABLE marker(id INTEGER); ATTACH ':memory:' AS other;",
            "CREATE TABLE marker(id INTEGER); PRAGMA writable_schema = ON;",
            "CREATE TABLE marker(id INTEGER); UPDATE _rupa_migrations SET owner = NULL;",
            "CREATE TABLE marker(id INTEGER); CREATE TRIGGER hijack AFTER INSERT ON _rupa_migrations BEGIN DELETE FROM marker; END;",
        ]
    }
}

fn memory_runner() -> SqliteExecutor {
    SqliteExecutor::open_in_memory().expect("SQLite connection")
}

#[test]
fn conformance() {
    sync::run(&mut DatabaseFile::new());
}

#[test]
fn conformance_async_through_blocking() {
    asynchronous::run(&mut Blocked(DatabaseFile::new()));
}
