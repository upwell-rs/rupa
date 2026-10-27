use std::marker::PhantomData;

use rupa_migrate::commands::Exit;
use rupa_migrate::commands::cli::Migrate;
use upwell::clap::{self, ArgMatches, Args, FromArgMatches};
use upwell::{BootstrapContext, BootstrapState, PluginCliCommand, PluginCommandContext, Setup};

use super::{CommandError, Declaration, Mode, Settings};

/// Database `D`'s parsed migration command, run at Setup: the configuration
/// is loaded and nothing is built or started.
///
/// It connects as `D`'s migration settings say; `--url` (or
/// `DATABASE_URL`) replaces the configured URL.
pub struct Command<D> {
    migrate: Migrate,
    declaration: PhantomData<fn() -> D>,
}

impl<D> Command<D> {
    /// The command `migrate` describes, on `D`.
    pub const fn new(migrate: Migrate) -> Self {
        Self {
            migrate,
            declaration: PhantomData,
        }
    }
}

impl<D> FromArgMatches for Command<D> {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        Migrate::from_arg_matches(matches).map(Self::new)
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        self.migrate.update_from_arg_matches(matches)
    }
}

impl<D> Args for Command<D> {
    fn augment_args(command: clap::Command) -> clap::Command {
        Migrate::augment_args(command)
    }

    fn augment_args_for_update(command: clap::Command) -> clap::Command {
        Migrate::augment_args_for_update(command)
    }
}

impl<D: Declaration> PluginCliCommand for Command<D> {
    type Phase = Setup;
    type Error = CommandError;

    async fn run(&self, mut context: PluginCommandContext<Setup>) -> Result<(), CommandError> {
        let catalog = D::catalog()?;
        let settings = configured::<D>(context.bootstrap_mut())?;
        let config = match (settings.map(|s| s.migration()), &self.migrate.url) {
            (Some(Ok(mut config)), Some(url)) => {
                config.url.clone_from(url);

                Ok(config)
            }
            (Some(Ok(config)), None) => Ok(config),
            (None, Some(url)) => Ok(rupa_core::config::DbConfig::new(url.as_str())),
            (Some(Err(error)), _) => Err(error.to_string()),
            (None, None) => Err(format!(
                "no database is configured at `{}`; configure one or pass --url",
                D::CONFIG
            )),
        };
        let migrate = Migrate {
            url: config.as_ref().ok().map(|config| config.url.clone()),
            ..self.migrate.clone()
        };
        let exit =
            D::Mode::execute(&migrate, &catalog, config.as_ref().map_err(String::as_str)).await;

        match exit {
            Exit::Success => Ok(()),
            exit => Err(CommandError::Unsuccessful(exit)),
        }
    }
}

/// `D`'s settings from the loaded configuration: none when the application
/// loads no configuration or `D`'s path holds none.
fn configured<D: Declaration>(
    context: &mut BootstrapContext,
) -> Result<Option<Settings<D>>, CommandError> {
    let config = context
        .bootstrap_mut()
        .and_then(BootstrapState::take_config);
    let settings = match config {
        Some(config) if config.contains(D::CONFIG) => {
            let settings = config
                .get_config::<Settings<D>>(D::CONFIG)
                .map_err(|source| CommandError::Config {
                    path: D::CONFIG,
                    source,
                })?;

            Some(settings)
        }
        _ => None,
    };

    Ok(settings)
}
