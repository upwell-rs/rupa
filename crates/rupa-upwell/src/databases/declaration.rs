use rupa_core::pool::Connect;
use rupa_migrate::{Catalog, MigrationInfo, ValidationError};

use super::Mode;

/// One database an Upwell application uses, as [`database!`](crate::database!)
/// declares it: its pool, configuration path, migrations and command.
pub trait Declaration: Send + Sync + Sized + 'static {
    /// The driver pool the application uses.
    type Pool: Connect;
    /// The catalog's migration type.
    type Migration: MigrationInfo + Send + Sync + 'static;
    /// How the pool opens and runs migrations: its `Connect::Completion`.
    type Mode: Mode<Self::Pool, Self::Migration>;

    /// The component's display name.
    const NAME: &'static str;
    /// The component's stable identity.
    const ID: &'static str;
    /// Where its [`DatabaseSettings`](super::DatabaseSettings) live.
    const CONFIG: &'static str;

    /// The migration catalog, as rupa-build generates it.
    fn catalog() -> Result<Catalog<'static, Self::Migration>, ValidationError>;
}
