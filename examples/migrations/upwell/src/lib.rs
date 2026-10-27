//! An Upwell application whose command line runs the `db` crate's migration
//! commands: `migrate` for the numbered `primary` stream and `reporting` for
//! the timestamped one. Both run at Setup, before anything is built.

use rupa::drivers::sqlite::{SqliteError, SqliteExecutor};
use rupa::migrate::{Catalog, ValidationError};
use rupa::upwell::migrations::sync::{Migrations, Target};
use upwell::{PluginId, app, namespaced_id};

/// The numbered `primary` stream, configured at `migrations.primary`.
pub struct Primary;

impl Target for Primary {
    type Migration = db::migrations::Migrations;
    type Session = SqliteExecutor;
    type Error = SqliteError;

    const PLUGIN: PluginId = namespaced_id!(PluginId, "example/primary-migrations");
    const COMMAND: &'static str = "migrate";
    const CONFIG: &'static str = "migrations.primary";

    fn catalog() -> Result<Catalog<'static, Self::Migration>, ValidationError> {
        db::migrations::catalog()
    }

    fn connect(url: Option<&str>) -> Result<Self::Session, Self::Error> {
        open(url)
    }
}

/// The timestamped `reporting` stream, configured at `migrations.reporting`.
pub struct Reporting;

impl Target for Reporting {
    type Migration = db::reporting::Migrations;
    type Session = SqliteExecutor;
    type Error = SqliteError;

    const PLUGIN: PluginId = namespaced_id!(PluginId, "example/reporting-migrations");
    const COMMAND: &'static str = "reporting";
    const CONFIG: &'static str = "migrations.reporting";

    fn catalog() -> Result<Catalog<'static, Self::Migration>, ValidationError> {
        db::reporting::catalog()
    }

    fn connect(url: Option<&str>) -> Result<Self::Session, Self::Error> {
        open(url)
    }
}

/// The SQLite file `url` names, or an in-memory database without one.
fn open(url: Option<&str>) -> Result<SqliteExecutor, SqliteError> {
    match url {
        Some(path) => SqliteExecutor::open(path),
        None => SqliteExecutor::open_in_memory(),
    }
}

app! {
    /// The example application: migration commands only, no service.
    pub app Example {
        name: "rupa-example-upwell",
        protocol: (),
        plugins: [Migrations<Primary>, Migrations<Reporting>],
    }
}
