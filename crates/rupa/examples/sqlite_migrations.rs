//! Mixed embedded SQL and compiled Rust migrations over a dedicated connection.

mod migrations;

use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::{Catalog, VersionScheme};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let entries = migrations::entries();
    let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries)?;
    let connection = match std::env::args_os().nth(1) {
        Some(path) => SqliteExecutor::open(path)?,
        None => SqliteExecutor::open_in_memory()?,
    };
    let report = catalog.apply(connection)?;

    for version in report.applied {
        println!("applied {version}");
    }

    Ok(())
}
