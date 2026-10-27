//! Link-time registration: `database!` places each database's descriptors in
//! Upwell's discovered slices, and its migration command in [`COMMANDS`].

use upwell::schema::config_value_schema;
use upwell::{
    ComponentDescriptor, ConfigBindingDescriptor, ConfigProperties, ContributionId, Plugin,
    PluginCliRegistrar, PluginContributions, PluginId, ProviderDescriptor, ProviderOf,
    TypeDescriptor, factories, linkme, namespaced_id,
};

use super::component::build;
use super::{Command, Database, Declaration, Settings};

/// Every declared database's migration command, collected at link time.
#[linkme::distributed_slice]
#[linkme(crate = upwell::linkme)]
pub static COMMANDS: [CommandRegistration];

/// One declared database's migration command.
pub struct CommandRegistration {
    /// The command name.
    pub name: &'static str,
    /// Its contribution identity in the [`Migrations`] plugin.
    pub id: ContributionId,
    /// Registers the command's parser and runner.
    pub register: fn(&mut PluginCliRegistrar, ContributionId, &'static str),
}

/// Adds every declared database's migration command to the application's
/// command line. Register it once in the application's `plugins`; the
/// databases themselves need no registration.
#[derive(Debug, Default)]
pub struct Migrations;

impl Plugin for Migrations {
    const ID: PluginId = namespaced_id!(PluginId, "rupa/migrations");

    fn contribute(self, _contributions: &mut PluginContributions) {}

    fn cli(&self, cli: &mut PluginCliRegistrar) {
        let mut commands: Vec<_> = COMMANDS.iter().collect();

        commands.sort_by_key(|command| command.name);

        for command in commands {
            (command.register)(cli, command.id, command.name);
        }
    }
}

/// The configuration binding of database `D`, at its path.
pub const fn settings<D: Declaration>() -> ConfigBindingDescriptor {
    ConfigBindingDescriptor {
        ty: TypeDescriptor::of::<Settings<D>>(D::NAME),
        path: D::CONFIG,
        bind: <Settings<D> as ConfigProperties>::bind,
        slot: <Settings<D> as ConfigProperties>::slot,
        defaults: <Settings<D> as ConfigProperties>::DEFAULTS,
        describe: config_value_schema::<Settings<D>>,
    }
}

/// The singleton component of database `D`, built by its gated factory.
pub const fn component<D: Declaration>() -> ComponentDescriptor {
    ComponentDescriptor::of::<Database<D>>()
        .with_factories(factories!["rupa/database" => build::<D>])
}

/// The provider of `R`, a repository trait object named `name`, over
/// database `D`.
pub const fn provider<D: Declaration, R: ?Sized + Send + Sync + 'static>(
    name: &'static str,
) -> ProviderDescriptor
where
    Database<D>: ProviderOf<R>,
{
    ProviderDescriptor::of::<Database<D>, R>(name)
}

/// Registers database `D`'s migration command as `name`.
pub fn command<D: Declaration>(
    cli: &mut PluginCliRegistrar,
    id: ContributionId,
    name: &'static str,
) {
    cli.command::<Command<D>>(id, name);
}
