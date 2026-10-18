use rupa::drivers::sqlite::SqlitePool;
use rupa::{DbConfig, Executor, Pool, raw};

pub fn open(schema: &'static str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(&DbConfig::new("sqlite::memory:"))?;
    pool.get()?.run(raw().sql(schema).execute::<u64>())?;

    Ok(pool)
}
