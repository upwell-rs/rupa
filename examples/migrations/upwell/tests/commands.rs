//! The `db` crate's migration commands, run through the example Upwell
//! application's generated command line.

mod support;

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use rupa::migrate::commands::Exit;
use rupa::upwell::exit_code;
use rupa_example_upwell::Example;
use support::{Scratch, pending};
use upwell::component;

/// How often the application built [`Marker`].
static BUILDS: AtomicUsize = AtomicUsize::new(0);

/// A component every build of the application constructs; the build test
/// in `build.rs` proves it.
#[component(factory = build_marker)]
pub struct Marker;

async fn build_marker() -> Marker {
    BUILDS.fetch_add(1, Ordering::SeqCst);

    Marker
}

#[tokio::test]
async fn the_configured_database_migrates_without_building_the_application() {
    let scratch = Scratch::new("configured");
    let config = scratch.config_dir(&scratch.toml("check"));

    let applied = Example::run_with(["app", "--config", &config, "migrate", "apply"]).await;

    assert!(applied.is_ok(), "apply succeeds: {applied:?}");
    assert_eq!(pending(&scratch.database("primary.db")), 0);
    assert_eq!(BUILDS.load(Ordering::SeqCst), 0, "nothing is built");
}

#[tokio::test]
async fn a_url_on_the_command_line_overrides_the_configured_one() {
    let scratch = Scratch::new("override");
    let config = scratch.config_dir(&scratch.toml("check"));
    let chosen = scratch.database("chosen.db");
    let url = format!("sqlite://{chosen}");

    let applied = Example::run_with([
        "app", "--config", &config, "migrate", "apply", "--url", &url,
    ])
    .await;

    assert!(applied.is_ok(), "apply succeeds: {applied:?}");
    assert_eq!(pending(&chosen), 0);
    assert!(!std::path::Path::new(&scratch.database("primary.db")).exists());
}

#[tokio::test]
async fn a_refused_command_ends_with_rupas_exit_status() {
    let scratch = Scratch::new("refused");
    let config = scratch.config_dir(&scratch.toml("check"));

    let reverted = Example::run_with([
        "app",
        "--config",
        &config,
        "reporting",
        "revert",
        "--last",
        "1",
    ])
    .await;

    assert_eq!(exit_code(reverted), ExitCode::from(Exit::Blocked));
}

#[tokio::test]
async fn invalid_database_configuration_is_a_usage_error() {
    let scratch = Scratch::new("invalid");
    let config =
        scratch.config_dir("[database.primary]\ndriver = \"sqlite\"\nfile = \"typo.db\"\n");

    let status = Example::run_with(["app", "--config", &config, "migrate", "status"]).await;

    assert_eq!(exit_code(status), ExitCode::from(Exit::Usage));
}
