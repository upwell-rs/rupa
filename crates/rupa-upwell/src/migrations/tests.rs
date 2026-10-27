use std::process::ExitCode;

use rupa_migrate::ValidationError;
use rupa_migrate::commands::Exit;
use upwell_app::CliError;

use super::*;

fn failed(error: CommandError) -> Result<(), CliError> {
    Err(CliError::Command(upwell_app::CommandError::new(
        "migrate apply",
        error,
    )))
}

#[test]
fn a_successful_outcome_finishes_the_command() {
    assert!(finish(Exit::Success).is_ok());
    assert!(matches!(
        finish(Exit::Busy),
        Err(CommandError::Unsuccessful(Exit::Busy))
    ));
}

#[test]
fn command_errors_end_with_rupa_exit_statuses() {
    let catalog = CommandError::Catalog(ValidationError::InvalidStream);

    assert_eq!(catalog.exit(), Exit::Blocked);
    assert_eq!(
        CommandError::Unsuccessful(Exit::Uncertain).exit(),
        Exit::Uncertain
    );
    assert_eq!(
        exit_code(failed(CommandError::Unsuccessful(Exit::Migration))),
        ExitCode::from(Exit::Migration)
    );
    assert_eq!(exit_code(failed(catalog)), ExitCode::from(Exit::Blocked));
}

#[test]
fn other_application_failures_end_as_failed() {
    let error = CliError::Output(std::io::Error::other("closed"));

    assert_eq!(exit_code(Ok(())), ExitCode::SUCCESS);
    assert_eq!(exit_code(Err(error)), ExitCode::from(Exit::Failed));
}

#[test]
fn a_url_on_the_command_line_wins_over_configuration() {
    let migrate = Migrate {
        operation: rupa_migrate::commands::cli::Operation::Status,
        format: rupa_migrate::commands::Format::Human,
        url: Some(String::from("cli.db")),
    };
    let mut context = BootstrapContext::new(upwell_app::ExecutionMode::Run);
    let resolved = resolve(&migrate, &mut context, "rupa.migrations").expect("resolves");

    assert!(matches!(resolved, Cow::Borrowed(_)));
    assert_eq!(resolved.url.as_deref(), Some("cli.db"));
}

#[test]
fn without_loaded_configuration_no_url_is_configured() {
    let migrate = Migrate {
        operation: rupa_migrate::commands::cli::Operation::Status,
        format: rupa_migrate::commands::Format::Human,
        url: None,
    };
    let mut context = BootstrapContext::new(upwell_app::ExecutionMode::Run);
    let resolved = resolve(&migrate, &mut context, "rupa.migrations").expect("resolves");

    assert_eq!(resolved.url, None);
}
