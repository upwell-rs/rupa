use std::error::Error;
use std::io::Write as _;
use std::process::ExitCode;

use rupa_migrate::ValidationError;
use rupa_migrate::commands::Exit;
use upwell_app::CliError;
use upwell_config::ConfigError;

/// Why a migration command did not succeed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CommandError {
    /// The target's configuration could not be read.
    #[error("cannot read the migration target configuration at `{path}`: {source}")]
    Config {
        /// The target's configuration path.
        path: &'static str,
        /// The cause.
        #[source]
        source: ConfigError,
    },
    /// The target's catalog is invalid.
    #[error("invalid migration catalog: {0}")]
    Catalog(#[from] ValidationError),
    /// The command ran and printed an outcome that was not a success.
    #[error("migration command ended with status {}", .0.as_str())]
    Unsuccessful(Exit),
}

impl CommandError {
    /// The exit status this error ends the process with: the printed
    /// outcome's, [`Exit::Usage`] for configuration and [`Exit::Blocked`] for
    /// an invalid catalog.
    pub const fn exit(&self) -> Exit {
        match self {
            Self::Config { .. } => Exit::Usage,
            Self::Catalog(_) => Exit::Blocked,
            Self::Unsuccessful(exit) => *exit,
        }
    }
}

/// The process exit status of an Upwell application run, for its `main`.
///
/// A migration command's failure ends with rupa's stable [`Exit`] status,
/// and any other failure with [`Exit::Failed`]. The error is written to
/// stderr unless the command already printed its outcome. A failed write is
/// ignored: the status still reports the failure.
pub fn exit_code(result: Result<(), CliError>) -> ExitCode {
    let Err(error) = result else {
        return ExitCode::SUCCESS;
    };
    let command = command_error(&error);

    if !matches!(command, Some(CommandError::Unsuccessful(_))) {
        let _ = writeln!(std::io::stderr().lock(), "error: {error}");
    }

    command.map_or(Exit::Failed, CommandError::exit).into()
}

/// The migration command error in `error`'s source chain.
fn command_error(error: &CliError) -> Option<&CommandError> {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);

    while let Some(error) = current {
        if let Some(command) = error.downcast_ref::<CommandError>() {
            return Some(command);
        }

        current = error.source();
    }

    None
}
