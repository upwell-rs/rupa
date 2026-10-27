//! Migration commands in an Upwell application's command line.
//!
//! A [`sync::Target`] or [`asynchronous::Target`] names one migration stream:
//! its catalog, its command name, where its configuration lives and how to
//! connect. Registering its [`sync::Migrations`] or
//! [`asynchronous::Migrations`] plugin adds rupa's operator commands under
//! that name, such as `app migrate status` or `app migrate apply --through 4`.
//!
//! The commands run at Upwell's Setup phase: the application's configuration
//! is loaded, and no component is constructed and no service, listener or job
//! starts. Each one prints its outcome like rupa's standalone command line,
//! and [`exit_code`] turns the application's result into rupa's stable exit
//! status.
//!
//! The connection URL comes from `--url` or `DATABASE_URL`, and otherwise
//! from `url` at the target's configuration path. Keep it apart from the
//! credentials the running application uses. Its passwords never appear in
//! output.

mod error;
mod target;

use std::borrow::Cow;

use rupa_migrate::commands::Exit;
use rupa_migrate::commands::cli::Migrate;
use serde::Deserialize;
use upwell_app::{BootstrapContext, BootstrapState, ContributionId, namespaced_id};

pub use error::{CommandError, exit_code};

/// The identity of the command each target plugin contributes; plugins scope
/// it, so every target can use it.
const COMMAND_ID: ContributionId = namespaced_id!(ContributionId, "rupa/migrate");

/// What a target's configuration path holds.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TargetConfig {
    url: Option<String>,
}

#[cfg(feature = "sync")]
target::mode!(sync [] []
    [Result<Self::Session, Self::Error>]
    [T::connect]);

#[cfg(feature = "async")]
target::mode!(asynchronous [.await] [+ Send + Sync]
[impl std::future::Future<Output = Result<Self::Session, Self::Error>> + Send]
[|url: Option<&str>| {
    let url = url.map(str::to_owned);

    async move { T::connect(url.as_deref()).await }
}]);

/// `migrate`, with the URL configured at `path` when it names none.
fn resolve<'m>(
    migrate: &'m Migrate,
    context: &mut BootstrapContext,
    path: &'static str,
) -> Result<Cow<'m, Migrate>, CommandError> {
    if migrate.url.is_some() {
        return Ok(Cow::Borrowed(migrate));
    }

    let resolved = match configured(context, path)?.url {
        Some(url) => Cow::Owned(Migrate {
            url: Some(url),
            ..migrate.clone()
        }),
        None => Cow::Borrowed(migrate),
    };

    Ok(resolved)
}

/// The target configuration at `path`: empty when the application loads no
/// configuration or `path` holds none.
fn configured(
    context: &mut BootstrapContext,
    path: &'static str,
) -> Result<TargetConfig, CommandError> {
    let config = context
        .bootstrap_mut()
        .and_then(BootstrapState::take_config);
    let target = match config {
        Some(config) if config.contains(path) => config
            .get(path)
            .map_err(|source| CommandError::Config { path, source })?,
        _ => TargetConfig::default(),
    };

    Ok(target)
}

/// Success, or the exit status the printed outcome ended with.
fn finish(exit: Exit) -> Result<(), CommandError> {
    match exit {
        Exit::Success => Ok(()),
        exit => Err(CommandError::Unsuccessful(exit)),
    }
}

#[cfg(test)]
mod tests;
