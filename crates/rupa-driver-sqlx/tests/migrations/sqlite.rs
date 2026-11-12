use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rupa::migrate::Policy;
use rupa_conformance::migrations::asynchronous;
use rupa_driver_sqlx::SqlxExecutor;
use tokio::runtime::Runtime;

use super::{Spec, Sqlx, interop};

const SPEC: Spec = Spec {
    reset: "",
    table_exists: "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
    forbidden: &[
        "CREATE TABLE marker(id INTEGER); ATTACH ':memory:' AS other;",
        "CREATE TABLE marker(id INTEGER); PRAGMA writable_schema = ON;",
        "CREATE TABLE marker(id INTEGER); UPDATE _rupa_migrations SET owner = NULL;",
        "CREATE TABLE marker(id INTEGER); CREATE TRIGGER hijack AFTER INSERT ON _rupa_migrations BEGIN DELETE FROM marker; END;",
    ],
    outside: "",
};

/// A fresh database file's path.
fn database() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    std::env::temp_dir().join(format!(
        "rupa-sqlx-migrations-{}-{}.sqlite",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display()).replace('\\', "/")
}

/// The database file `url` names.
fn path(url: &str) -> PathBuf {
    PathBuf::from(
        url.trim_start_matches("sqlite://")
            .trim_end_matches("?mode=rwc"),
    )
}

/// Replaces the target's file with a fresh one.
fn fresh(target: &mut Sqlx<sqlx::Sqlite>) {
    remove(&target.url);
    target.url = url(&database());
}

fn remove(url: &str) {
    let file = path(url);

    if file.exists() {
        std::fs::remove_file(&file).expect("remove temporary database");
    }
}

#[test]
fn sqlite() {
    let mut target = Sqlx::<sqlx::Sqlite> {
        url: url(&database()),
        runtime: Runtime::new().expect("tokio runtime"),
        spec: SPEC,
        reconnect: fresh,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        asynchronous::run(&mut target);

        eprintln!("migration interop sqlite");
        rupa_conformance::migrations::Target::reset(&mut target);
        interop::run(
            || {
                rupa_driver_rusqlite::SqliteExecutor::open(path(&target.url))
                    .expect("open")
                    .with_migration_lock_timeout(Duration::from_millis(250))
            },
            || {
                target
                    .runtime
                    .block_on(SqlxExecutor::<sqlx::Sqlite>::connect(&target.url))
                    .expect("connect")
                    .with_migration_lock_timeout(Duration::from_millis(250))
            },
            &target.runtime,
            Policy::TransactionalRequired,
        );
    }));

    remove(&target.url);

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
