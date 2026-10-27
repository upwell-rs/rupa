/// Emits one mode's `Target`, `Migrations` plugin and `Command`.
macro_rules! mode {
    ($mode:ident [$($await:tt)*] [$($bound:tt)*] [$($connect:tt)*] [$($adapt:tt)*]) => {
        pub mod $mode {
            //! Targets on this mode's sessions.

            use std::marker::PhantomData;

            use rupa_migrate::commands::cli::{Migrate, $mode::execute};
            use rupa_migrate::$mode::{Migration, Session};
            use rupa_migrate::{Catalog, ValidationError};
            use upwell_app::clap::{self, ArgMatches, Args, FromArgMatches};
            use upwell_app::{
                Plugin, PluginCliCommand, PluginCliRegistrar, PluginCommandContext,
                PluginContributions, PluginId, Setup,
            };

            use super::{COMMAND_ID, CommandError, finish, resolve};

            /// One migration stream whose commands an Upwell application runs.
            pub trait Target: Send + Sync + 'static {
                /// The catalog's migration type.
                type Migration: Migration<Self::Session> $($bound)*;
                /// The session commands run on.
                type Session: Session;
                /// Why connecting failed.
                type Error: std::error::Error + Send + Sync + 'static;

                /// The plugin's identity, unique in the application.
                const PLUGIN: PluginId;
                /// The command name, such as `migrate`.
                const COMMAND: &'static str;
                /// The configuration path holding the target's `url`, apart
                /// from the running application's credentials.
                const CONFIG: &'static str;

                /// The stream's catalog, as rupa-build generates it.
                fn catalog() -> Result<Catalog<'static, Self::Migration>, ValidationError>;

                /// Opens a session on the target `url` names: `--url`,
                /// `DATABASE_URL` or the configured one, if any. Called only
                /// by a command that reads or changes the target.
                fn connect(url: Option<&str>) -> $($connect)*;
            }

            /// Adds `T`'s migration commands to the application's command
            /// line. Register it in the application's `plugins`.
            pub struct Migrations<T>(PhantomData<fn() -> T>);

            impl<T> Default for Migrations<T> {
                fn default() -> Self {
                    Self(PhantomData)
                }
            }

            impl<T: Target> Plugin for Migrations<T> {
                const ID: PluginId = T::PLUGIN;

                fn contribute(self, _contributions: &mut PluginContributions) {}

                fn cli(&self, cli: &mut PluginCliRegistrar) {
                    cli.command::<Command<T>>(COMMAND_ID, T::COMMAND);
                }
            }

            /// `T`'s parsed migration command, run at Setup: the
            /// configuration is loaded and nothing is built or started.
            pub struct Command<T> {
                migrate: Migrate,
                target: PhantomData<fn() -> T>,
            }

            impl<T> Command<T> {
                /// The command `migrate` describes, on `T`.
                pub const fn new(migrate: Migrate) -> Self {
                    Self {
                        migrate,
                        target: PhantomData,
                    }
                }
            }

            impl<T> FromArgMatches for Command<T> {
                fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
                    Migrate::from_arg_matches(matches).map(Self::new)
                }

                fn update_from_arg_matches(
                    &mut self,
                    matches: &ArgMatches,
                ) -> Result<(), clap::Error> {
                    self.migrate.update_from_arg_matches(matches)
                }
            }

            impl<T> Args for Command<T> {
                fn augment_args(command: clap::Command) -> clap::Command {
                    Migrate::augment_args(command)
                }

                fn augment_args_for_update(command: clap::Command) -> clap::Command {
                    Migrate::augment_args_for_update(command)
                }
            }

            impl<T: Target> PluginCliCommand for Command<T> {
                type Phase = Setup;
                type Error = CommandError;

                async fn run(
                    &self,
                    mut context: PluginCommandContext<Setup>,
                ) -> Result<(), CommandError> {
                    let catalog = T::catalog()?;
                    let migrate = resolve(&self.migrate, context.bootstrap_mut(), T::CONFIG)?;
                    let exit = execute(&migrate, &catalog, $($adapt)*) $($await)*;

                    finish(exit)
                }
            }
        }
    };
}

pub(super) use mode;
