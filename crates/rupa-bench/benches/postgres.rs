//! Round trips to a real Postgres. Skipped unless `RUPA_BENCH_POSTGRES_URL`
//! names a database the benchmark may write to, e.g.
//!
//! ```text
//! docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:17
//! RUPA_BENCH_POSTGRES_URL=postgres://postgres:postgres@localhost/postgres cargo bench -p rupa-bench --bench postgres
//! ```
//!
//! `executor` vs `raw_client` is rupa's overhead over tokio-postgres (the
//! raw client prepares once, like rupa's statement cache); `repo_database`
//! adds the repository, type erasure and the pool.

use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use rupa::prelude::*;
use rupa::{AsyncExecutor as _, AsyncPool as _, Database, DbConfig};
use rupa_bench::{POSTGRES_DDL, User, Users, new_users};
use rupa_driver_tokio_postgres::PgPool;

fn postgres(c: &mut Criterion) {
    let Ok(url) = std::env::var("RUPA_BENCH_POSTGRES_URL") else {
        eprintln!("RUPA_BENCH_POSTGRES_URL is not set; skipping the Postgres benchmarks");
        return;
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pool = rt.block_on(async {
        let pool = PgPool::connect(&DbConfig::new(&url).max_connections(4))
            .await
            .expect("connect");
        let mut conn = pool.get().await.unwrap();
        conn.client().batch_execute(POSTGRES_DDL).await.unwrap();
        conn.run(insert::<User>().values(&new_users(1_000)).affected())
            .await
            .unwrap();
        pool
    });

    let mut g = c.benchmark_group("postgres");
    let mut conn = rt.block_on(pool.get()).unwrap();
    g.bench_function("get_by_id/executor", |b| {
        b.iter(|| rt.block_on(conn.run(get::<User>(&42))).unwrap().unwrap())
    });
    let raw = rt
        .block_on(
            conn.client()
                .prepare("SELECT id, email, name, active, prefs FROM users WHERE id = $1"),
        )
        .unwrap();
    g.bench_function("get_by_id/raw_client", |b| {
        b.iter(|| {
            rt.block_on(conn.client().query_one(&raw, &[&42i64]))
                .unwrap()
        })
    });
    drop(conn);

    let users: Arc<dyn Users> = Arc::new(Repo::new(Database::new(pool)));
    g.bench_function("get_by_id/repo_database", |b| {
        b.iter(|| rt.block_on(users.by_id(42)).unwrap().unwrap())
    });
    g.finish();
}

criterion_group!(benches, postgres);
criterion_main!(benches);
