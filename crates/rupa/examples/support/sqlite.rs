use rupa::drivers::sqlite::SqlitePool;
use rupa::{DbConfig, Pool, raw};

pub fn open(schema: &'static str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(&DbConfig::new("sqlite::memory:"))?;
    raw()
        .sql(schema)
        .execute::<u64>()
        .execute(&mut pool.get()?)?;

    Ok(pool)
}
