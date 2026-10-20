//! Applies the `db` crate's generated migration catalog on a dedicated connection.

use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::sync::Apply;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = db::migrations::catalog()?;
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
