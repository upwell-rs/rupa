//! The timestamped `reporting` stream, through the operator commands an
//! application's command line runs.

use std::path::PathBuf;

use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::commands::sync::run;
use rupa::migrate::commands::{Command, Exit, Format, Redactor, Report};
use rupa::migrate::{ApplyOptions, RevertTarget, VersionScheme};
use rupa_example_db::reporting::{SCHEME, catalog};

/// A database file for one test, removed when dropped.
struct Database(PathBuf);

impl Database {
    /// A leftover file from an earlier run may not exist, so its removal
    /// error is ignored.
    fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rupa-example-{}-{test}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);

        Self(path)
    }
}

impl Drop for Database {
    /// Cleanup is best effort: a failure must not mask the test's own.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn the_timestamped_stream_applies_and_reverts_through_the_commands() {
    let catalog = catalog().expect("generated catalog validates");
    let database = Database::new("reporting");
    let connect = || SqliteExecutor::open(&database.0);

    let planned = run(&catalog, &Command::Plan(ApplyOptions::DEFAULT), connect);
    let applied = run(&catalog, &Command::Apply(ApplyOptions::DEFAULT), connect);
    let reverted = run(&catalog, &Command::Revert(RevertTarget::Last(1)), connect);
    let status = run(&catalog, &Command::Status, connect);
    let json: serde_json::Value =
        serde_json::from_str(&status.render(Format::Json, &Redactor::new())).expect("JSON");

    assert_eq!(SCHEME, VersionScheme::UtcTimestamp);
    assert!(
        matches!(&planned.report, Some(Report::Plan(plan)) if plan.steps.len() == 2),
        "{planned:?}"
    );
    assert!(
        matches!(
            &applied.report,
            Some(Report::Applied(report)) if report.applied.iter().map(ToString::to_string).eq(["20261027080000", "20261027081753"])
        ),
        "the SQL migration, then the Rust one: {applied:?}"
    );
    assert!(
        matches!(
            &reverted.report,
            Some(Report::Reverted(report)) if report.reverted.iter().map(ToString::to_string).eq(["20261027081753"])
        ),
        "{reverted:?}"
    );
    assert_eq!(status.exit(), Exit::Success);
    assert_eq!(json["scheme"], "utc-timestamp");
    assert_eq!(
        json["report"]["pending"][0]["source"],
        "20261027081753_index_event_kinds.rs"
    );
}
