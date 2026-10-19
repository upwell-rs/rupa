#![cfg(all(feature = "migrations", feature = "sqlite-unbundled"))]

mod application;

use application::entries;
use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::{Catalog, VersionScheme};

#[test]
fn mixed_sql_rust_apply_records_history_and_never_replays() {
    let entries = entries();
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");

    let session = SqliteExecutor::open_in_memory().expect("SQLite connection");
    let mut runner = session;

    let applied = runner.apply(&catalog).expect("mixed apply");

    assert_eq!(
        applied
            .applied
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert_eq!(
        runner
            .apply(&catalog)
            .expect("idempotent apply")
            .applied
            .len(),
        0
    );

    let history = runner.history().expect("installed history");
    assert_eq!(history.len(), 2);
    assert_eq!(
        history
            .iter()
            .map(|h| h.installation_rank)
            .collect::<Vec<_>>(),
        [1, 2]
    );

    let mut executor = runner;
    let title: String = executor
        .connection()
        .query_row("SELECT title FROM notes", [], |row| row.get(0))
        .expect("transformed title");
    assert_eq!(title, "HELLO; WORLD");
}

mod authorization;
mod concurrency;
mod failures;

use rupa::migrate::{
    ArtifactInput, Context, Error, Metadata, Migration, MigrationInfo, MigrationKind, Policy,
    Session,
};

/// A declared script fixture using the same immutable closure as its body.
#[derive(Clone, Copy)]
struct Body(&'static Metadata);

impl MigrationInfo for Body {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl<S: Session> Migration<S> for Body {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.script(self.0.inputs[0].contents)
    }
}

fn body(version: &'static str, sql: &'static str) -> Body {
    Body(Box::leak(Box::new(Metadata {
        version,
        description: "script fixture",
        policy: Policy::TransactionalRequired,
        kind: MigrationKind::Incremental,
        inputs: Box::leak(vec![ArtifactInput::new("body.sql", sql)].into_boxed_slice()),
    })))
}

fn memory_runner() -> SqliteExecutor {
    SqliteExecutor::open_in_memory().expect("SQLite connection")
}

/// A disposable file target shared by independent SQLite connections.
struct DatabaseFile(std::path::PathBuf);

impl DatabaseFile {
    fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rupa-migrations-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        Self(path)
    }

    fn session(&self) -> SqliteExecutor {
        SqliteExecutor::open(&self.0).expect("file connection")
    }
}

impl Drop for DatabaseFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).expect("remove temporary database");
    }
}
