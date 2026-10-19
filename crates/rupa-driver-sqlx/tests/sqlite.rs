//! The conformance suite on SQLite in memory (no Docker): on one executor,
//! then through a pooled, type-erased `Database`.

use rupa::core::tx::{AsyncTransactional, AsyncTxConnection};
use rupa::{DbConfig, DriverConfig, PoolConfig, Registry, SqliteOptions};
use rupa_conformance::{Harness, SQLITE_ATTACH, SQLITE_DDL};
use rupa_driver_sqlx::sqlite::{SqliteDriver, SqliteExecutor, SqliteManager, SqlitePool};

struct Sqlite(SqliteExecutor);

impl Harness for Sqlite {
    type Exec = SqliteExecutor;

    async fn reset(&mut self) {
        self.0.script(SQLITE_DDL).await.expect("reset schema");
    }

    fn exec(&mut self) -> &mut SqliteExecutor {
        &mut self.0
    }
}

#[tokio::test]
async fn conformance() {
    let mut exec = SqliteExecutor::connect("sqlite::memory:").await.unwrap();
    exec.script(SQLITE_ATTACH).await.unwrap();
    let mut h = Sqlite(exec);
    rupa_conformance::run(&mut h).await;
    rupa_conformance::run_transactions(&mut h).await;

    // Through a pool behind `Database`; the in-memory database gets one
    // connection, which attaches the conformance schema when opened.
    let manager = SqliteManager::from_config(&DbConfig::new("sqlite::memory:"))
        .unwrap()
        .init([SQLITE_ATTACH.to_owned()]);
    let pool = SqlitePool::new(manager, &PoolConfig::default())
        .await
        .unwrap();
    assert_eq!(
        pool.deadpool().status().max_size,
        1,
        "one in-memory database"
    );
    rupa_conformance::run_database(&rupa::Database::new(pool), SQLITE_DDL).await;
}

#[tokio::test]
async fn dropped_tx_marks_the_connection_dirty() {
    let mut exec = SqliteExecutor::connect("sqlite::memory:").await.unwrap();
    drop(exec.begin().await.unwrap());
    assert!(
        exec.is_dirty(),
        "drop cannot await, so the rollback is owed"
    );
    exec.clean().await.unwrap();
    assert!(!exec.is_dirty());
    // SQLite agrees: no transaction is open, so COMMIT is refused.
    assert!(exec.script("COMMIT").await.is_err());
}

#[tokio::test]
async fn registry_builds_a_pool_from_config() {
    let dir = std::env::temp_dir().join(format!("rupa-sqlx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.db");
    let _ = std::fs::remove_file(&file);
    let url = format!("sqlite://{}", file.display()).replace('\\', "/");

    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": url,
        "driver": "sqlx-sqlite",
        "journal_mode": "wal",
        "statement_cache_size": 4,
        "pool": { "max_size": 2 },
    }))
    .unwrap();
    let registry = Registry::new().with(SqliteDriver::new());
    assert_eq!(registry.resolve(&config).unwrap().name(), "sqlx-sqlite");
    let db = registry.connect(&config).await.unwrap();
    assert_eq!(db.status().max_size, 2);
    assert!(file.exists(), "rwc mode creates the file");
    drop(db);

    // A `sqlite` config (the rusqlite driver's name) is served by this one
    // when rusqlite isn't registered: same database, same options.
    let config = DbConfig::new(&url).driver(DriverConfig::Sqlite(SqliteOptions::default()));
    assert_eq!(registry.resolve(&config).unwrap().name(), "sqlx-sqlite");

    // Read-only on a missing file fails at connect.
    let missing = format!("sqlite://{}/missing.db?mode=ro", dir.display()).replace('\\', "/");
    assert!(registry.connect(&DbConfig::new(missing)).await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn statement_cache_size_is_configurable() {
    use rupa::AsyncPool;
    use rupa::core::ExprOps as _;
    use rupa_conformance::Note;

    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": "sqlite::memory:",
        "statement_cache_size": 2,
    }))
    .unwrap();
    let manager = SqliteManager::from_config(&config)
        .unwrap()
        .init([SQLITE_ATTACH.to_owned(), SQLITE_DDL.to_owned()]);
    let pool = SqlitePool::new(manager, &config.pool).await.unwrap();
    let mut conn = pool.get().await.unwrap();
    for n in 1..=4i64 {
        let ids: Vec<i64> = (0..n).collect();
        let q = rupa::core::select::<Note>().filter(Note::ID.in_(ids)).all();
        rupa::core::exec::AsyncExecutor::run(&mut conn, q)
            .await
            .unwrap();
    }

    assert_eq!(conn.cached_statements(), 2, "least recently used dropped");
}
