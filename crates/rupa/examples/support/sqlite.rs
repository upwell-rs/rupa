use rupa::drivers::sqlite::SqlitePool;
use rupa::{DbConfig, raw};

/// An in-memory SQLite pool with `schema`, `;`-separated statements, applied.
pub fn open(schema: &'static str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(&DbConfig::new("sqlite::memory:"))?;

    for statement in schema.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        raw().sql(statement).execute::<u64>().execute(&pool)?;
    }

    Ok(pool)
}
