//! Scratch directories and configurations shared by the example's tests.

#![allow(dead_code)]

use std::path::PathBuf;

use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::commands::sync::run;
use rupa::migrate::commands::{Command, Report};
use upwell::ConfigManager;
use upwell::config::Toml;

/// A scratch directory for one test, removed when dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    /// A leftover directory from an earlier run may not exist, so its
    /// removal error is ignored.
    pub fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("rupa-example-upwell-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);

        std::fs::create_dir_all(&path).expect("scratch directory is created");

        Self(path)
    }

    /// The path of database file `name` in this directory.
    pub fn database(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }

    /// Configuration for both databases in this directory: `primary.db`
    /// with `startup`, and `reporting.db` with startup off.
    pub fn toml(&self, startup: &str) -> String {
        format!(
            "[database.primary]\ndriver = \"sqlite\"\npath = {primary:?}\n\n\
             [database.primary.migrations]\nstartup = {startup:?}\n\n\
             [database.reporting]\ndriver = \"sqlite\"\npath = {reporting:?}\n\n\
             [database.reporting.migrations]\nstartup = \"off\"\n",
            primary = self.database("primary.db"),
            reporting = self.database("reporting.db"),
        )
    }

    /// A configuration directory holding `toml` as `application.toml`.
    pub fn config_dir(&self, toml: &str) -> String {
        let dir = self.0.join("config");

        std::fs::create_dir_all(&dir).expect("config directory is created");
        std::fs::write(dir.join("application.toml"), toml).expect("config is written");

        dir.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    /// Cleanup is best effort: a failure must not mask the test's own.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `toml` as an application configuration source.
pub fn source(toml: &str) -> ConfigManager<Toml> {
    ConfigManager::<Toml>::from_str(toml).expect("the configuration parses")
}

/// The pending versions of the `primary` stream at `database`.
pub fn pending(database: &str) -> usize {
    let catalog = db::migrations::catalog().expect("generated catalog validates");
    let outcome = run(&catalog, &Command::Status, || {
        SqliteExecutor::open(database)
    });

    match outcome.report {
        Some(Report::Status(status)) => status.pending.len(),
        other => panic!("expected a status report, got {other:?}"),
    }
}
