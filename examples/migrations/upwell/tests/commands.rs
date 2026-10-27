//! The `db` crate's migration commands, run through the example Upwell
//! application's generated command line.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::commands::sync::run;
use rupa::migrate::commands::{Command, Exit, Report};
use rupa::upwell::migrations::exit_code;
use rupa_example_upwell::Example;
use upwell::component;

/// How often the application built [`Marker`].
static BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A component the application would build before serving.
#[component(factory = build_marker)]
pub struct Marker;

async fn build_marker() -> Marker {
    BUILDS.fetch_add(1, Ordering::SeqCst);

    Marker
}

/// A scratch directory for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// A leftover directory from an earlier run may not exist, so its
    /// removal error is ignored.
    fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rupa-example-upwell-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);

        std::fs::create_dir_all(&path).expect("scratch directory is created");

        Self(path)
    }

    /// A configuration directory whose `primary` target is `database`.
    fn config(&self, database: &str) -> String {
        let dir = self.0.join("config");
        let toml = format!(
            "[migrations.primary]\nurl = {:?}\n",
            self.database(database)
        );

        std::fs::create_dir_all(&dir).expect("config directory is created");
        std::fs::write(dir.join("application.toml"), toml).expect("config is written");

        dir.to_string_lossy().into_owned()
    }

    fn database(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    /// Cleanup is best effort: a failure must not mask the test's own.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The pending versions of the `primary` stream at `database`.
fn pending(database: &str) -> usize {
    let catalog = db::migrations::catalog().expect("generated catalog validates");
    let outcome = run(&catalog, &Command::Status, || {
        SqliteExecutor::open(database)
    });

    match outcome.report {
        Some(Report::Status(status)) => status.pending.len(),
        other => panic!("expected a status report, got {other:?}"),
    }
}

#[tokio::test]
async fn the_configured_target_migrates_without_building_the_application() {
    let scratch = Scratch::new("configured");
    let config = scratch.config("primary.db");

    let applied = Example::run_with(["app", "--config", &config, "migrate", "apply"]).await;

    assert!(applied.is_ok(), "apply succeeds: {applied:?}");
    assert_eq!(pending(&scratch.database("primary.db")), 0);
    assert_eq!(BUILDS.load(Ordering::SeqCst), 0, "nothing is built");
}

#[tokio::test]
async fn a_url_on_the_command_line_overrides_the_configured_one() {
    let scratch = Scratch::new("override");
    let config = scratch.config("configured.db");
    let chosen = scratch.database("chosen.db");

    let applied = Example::run_with([
        "app", "--config", &config, "migrate", "apply", "--url", &chosen,
    ])
    .await;

    assert!(applied.is_ok(), "apply succeeds: {applied:?}");
    assert_eq!(pending(&chosen), 0);
    assert!(!std::path::Path::new(&scratch.database("configured.db")).exists());
}

#[tokio::test]
async fn a_refused_command_ends_with_rupas_exit_status() {
    let scratch = Scratch::new("refused");
    let database = scratch.database("reporting.db");

    let reverted = Example::run_with([
        "app",
        "reporting",
        "revert",
        "--last",
        "1",
        "--url",
        &database,
    ])
    .await;

    assert_eq!(exit_code(reverted), ExitCode::from(Exit::Blocked));
}

#[tokio::test]
async fn invalid_target_configuration_is_a_usage_error() {
    let scratch = Scratch::new("invalid");
    let dir = scratch.0.join("config");

    std::fs::create_dir_all(&dir).expect("config directory is created");
    std::fs::write(
        dir.join("application.toml"),
        "[migrations.primary]\nuri = \"typo.db\"\n",
    )
    .expect("config is written");

    let status = Example::run_with([
        "app",
        "--config",
        &dir.to_string_lossy(),
        "migrate",
        "status",
    ])
    .await;

    assert_eq!(exit_code(status), ExitCode::from(Exit::Usage));
}

#[tokio::test]
async fn building_the_application_builds_the_marker() {
    let prepared = Example::builder()
        .expect("the generated builder constructs")
        .prepare()
        .expect("the application prepares");
    let _app = prepared.build().await.expect("the application builds");

    assert_eq!(
        BUILDS.load(Ordering::SeqCst),
        1,
        "the marker proves a build"
    );
}
