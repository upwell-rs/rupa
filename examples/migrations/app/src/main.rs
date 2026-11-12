//! Runs the `db` crate's migration commands: one command line per stream,
//! each on its own SQLite database.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::commands::Exit;
use rupa::migrate::commands::cli::{Migrate, sync::execute};
use rupa::migrate::sync::Migration;
use rupa::migrate::{Catalog, ValidationError};

/// The application's command line.
#[derive(Debug, Parser)]
#[command(about = "Inspect and change the example application's databases")]
struct App {
    /// The stream to work on.
    #[command(subcommand)]
    stream: Stream,
}

/// The `db` crate's migration streams.
#[derive(Debug, Subcommand)]
enum Stream {
    /// Notes and tags: numbered SQL and Rust migrations, with a snapshot.
    Primary(Migrate),
    /// Usage events: SQL and Rust migrations versioned by UTC timestamps.
    Reporting(Migrate),
}

fn main() -> ExitCode {
    match App::parse().stream {
        Stream::Primary(migrate) => run(&migrate, db::migrations::catalog()),
        Stream::Reporting(migrate) => run(&migrate, db::reporting::catalog()),
    }
}

/// Runs `migrate` against `catalog` on the SQLite file `--url` names, or on
/// an in-memory database without one.
fn run<M: Migration<SqliteExecutor>>(
    migrate: &Migrate,
    catalog: Result<Catalog<'static, M>, ValidationError>,
) -> ExitCode {
    let catalog = match catalog {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("invalid migration catalog: {error}");

            return Exit::Blocked.into();
        }
    };

    execute(migrate, &catalog, |url| match url {
        Some(path) => SqliteExecutor::open(path),
        None => SqliteExecutor::open_in_memory(),
    })
    .into()
}
