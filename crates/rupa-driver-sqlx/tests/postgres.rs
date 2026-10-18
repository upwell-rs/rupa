//! The conformance suite against a real Postgres in Docker: on one executor,
//! then through the registry's pooled `Database`.

use rupa::core::tx::{AsyncTransactional, AsyncTxConnection};
use rupa::{AsyncPool, DbConfig, PoolConfig, Registry};
use rupa_conformance::{Harness, POSTGRES_DDL};
use rupa_driver_sqlx::postgres::{PgDriver, PgExecutor, PgPool};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

struct Pg(PgExecutor);

impl Harness for Pg {
    type Exec = PgExecutor;

    async fn reset(&mut self) {
        self.0.script(POSTGRES_DDL).await.expect("reset schema");
    }

    fn exec(&mut self) -> &mut PgExecutor {
        &mut self.0
    }
}

#[tokio::test]
async fn conformance() {
    let node = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );
    let mut h = Pg(PgExecutor::connect(&url).await.expect("connect"));
    rupa_conformance::run(&mut h).await;
    rupa_conformance::run_transactions(&mut h).await;
    dropped_tx_marks_the_connection_dirty(&mut h.0).await;

    // Again through the registry: a deadpool pool behind a type-erased
    // `Database`, selected by name since `postgres:` URLs have other drivers.
    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": url,
        "driver": "sqlx-postgres",
        "application_name": "rupa-sqlx-test",
        "pool": { "max_size": 3 },
    }))
    .unwrap();
    let db = Registry::new()
        .with(PgDriver::new())
        .connect(&config)
        .await
        .expect("pool");
    rupa_conformance::run_database(&db, POSTGRES_DDL).await;
    pooled_connections(&url).await;
}

/// The async-drop strategy, observed directly: a dropped `PgTx` leaves the
/// executor dirty until the rollback is sent.
async fn dropped_tx_marks_the_connection_dirty(exec: &mut PgExecutor) {
    exec.script(POSTGRES_DDL).await.unwrap();
    assert!(!exec.is_dirty());
    drop(exec.begin().await.unwrap());
    assert!(
        exec.is_dirty(),
        "drop cannot await, so the rollback is owed"
    );
    exec.clean().await.unwrap();
    assert!(!exec.is_dirty());
    // The server agrees: no transaction is open, so SAVEPOINT is refused.
    assert!(exec.script("SAVEPOINT probe").await.is_err());
}

/// Dirty connections are cleaned on return; options reach each connection.
async fn pooled_connections(url: &str) {
    use rupa::core::ExprOps as _;
    use rupa_conformance::Note;

    let pool = PgPool::connect(&DbConfig::new(url).pool(PoolConfig::default().max_size(1)))
        .await
        .unwrap();
    {
        let mut conn = pool.get().await.unwrap();
        drop(conn.begin().await.unwrap());
        assert!(conn.is_dirty());
    }
    let mut conn = pool.get().await.unwrap();
    assert!(!conn.is_dirty(), "recycling cleaned it");
    assert!(
        conn.script("SAVEPOINT probe").await.is_err(),
        "no transaction is open on the reused connection"
    );
    drop(conn);

    // `statement_cache_size` bounds each connection's prepared statements.
    let config: DbConfig =
        serde_json::from_value(serde_json::json!({ "url": url, "statement_cache_size": 2 }))
            .unwrap();
    let small = PgPool::connect(&config).await.unwrap();
    let mut conn = small.get().await.unwrap();
    conn.script(POSTGRES_DDL).await.unwrap();
    for n in 1..=4i64 {
        let ids: Vec<i64> = (0..n).collect();
        let q = rupa::core::select::<Note>()
            .filter(rupa::col!(Note::id).in_(ids))
            .all();
        rupa::core::exec::AsyncExecutor::run(&mut conn, q)
            .await
            .unwrap();
    }

    assert_eq!(conn.cached_statements(), 2, "least recently used dropped");

    // A wrong password fails at connect, with the server's error.
    let bad = url.replace("postgres:postgres@", "postgres:wrong@");
    let err = PgPool::connect(&DbConfig::new(bad)).await.unwrap_err();
    assert!(err.to_string().contains("password"), "{err}");
}
