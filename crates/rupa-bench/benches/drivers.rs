//! Every driver, the same workloads, the same databases: what the registry's
//! driver order means for performance.
//!
//! Postgres 17 and MySQL 8 run in testcontainers (or set
//! `RUPA_BENCH_POSTGRES_URL` / `RUPA_BENCH_MYSQL_URL` to use a server of your
//! own; the benchmark recreates a `users` table there). SQLite uses a WAL file
//! per driver in the temp directory. Every driver goes through the same
//! `Database` (pool of 8, type-erased) and the same `Arc<dyn Users>`
//! repository, so the differences between rows are the drivers.
//!
//! Two settings:
//!
//! - `async/<driver>`: called from a task on a multi-threaded tokio runtime,
//!   as in a web server. Sync drivers run there through `Blocking`, which
//!   blocks the worker thread for the duration of a statement.
//!   `postgres-sync` is absent: the `postgres` crate drives its own runtime
//!   and refuses to run inside tokio's.
//! - `sync/<driver>`: the sync drivers called from a plain thread, no runtime.
//!
//! Workloads: `get_by_id` (one row), `active_100` (100 rows decoded),
//! `insert_rollback` (begin, insert, rollback), and `concurrent_64`
//! (64 lookups at once; tasks in `async`, 8 threads × 8 in `sync`).
//!
//! `cargo bench -p rupa-bench --bench drivers` (Docker must be running), or
//! filter: `-- get_by_id`, `-- sqlx`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main};
use rupa::prelude::*;
use rupa::{AsyncExecutor as _, Database, DbConfig};
use rupa_bench::{MYSQL_DDL, NewUser, POSTGRES_DDL, Prefs, SQLITE_DDL, User, Users, new_users};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::{mysql::Mysql, postgres::Postgres};
use tokio::runtime::Runtime;

const ROWS: usize = 1_000;
const POOL: u32 = 8;
const CONCURRENT: u64 = 64;

/// One driver, connected and seeded.
struct Target {
    id: String,
    db: Database,
    users: Arc<dyn Users>,
    /// Whether it may run inside the tokio runtime.
    in_tokio: bool,
    /// Whether it may run without one.
    sync: bool,
}

/// Keeps the containers alive for the whole run.
#[derive(Default)]
struct Servers {
    _postgres: Option<ContainerAsync<Postgres>>,
    _mysql: Option<ContainerAsync<Mysql>>,
}

async fn connect(url: &str, driver: &str) -> Database {
    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": url,
        "driver": driver,
        "pool": { "max_size": POOL },
    }))
    .unwrap();
    // MySQL restarts once while initializing; retry for a while.
    for _ in 0..120 {
        match rupa::connect(&config).await {
            Ok(db) => return db,
            // A thread sleep, so this also works without a runtime.
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    panic!("cannot connect {driver} to {url}");
}

async fn seed(db: &Database, ddl: &'static str) {
    let mut conn = db.get().await.unwrap();
    for statement in ddl.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        conn.run(raw().sql(statement).execute::<u64>())
            .await
            .unwrap();
    }

    conn.run(insert::<User>().values(&new_users(ROWS)).affected())
        .await
        .unwrap();
}

async fn postgres_url(servers: &mut Servers) -> Option<String> {
    if let Ok(url) = std::env::var("RUPA_BENCH_POSTGRES_URL") {
        return Some(url);
    }

    let node = Postgres::default().with_tag("17-alpine").start().await;
    let node = node
        .map_err(|e| eprintln!("no Postgres container ({e}); skipping"))
        .ok()?;
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        node.get_host_port_ipv4(5432).await.unwrap()
    );
    servers._postgres = Some(node);
    Some(url)
}

async fn mysql_url(servers: &mut Servers) -> Option<String> {
    if let Ok(url) = std::env::var("RUPA_BENCH_MYSQL_URL") {
        return Some(url);
    }

    let node = Mysql::default().start().await;
    let node = node
        .map_err(|e| eprintln!("no MySQL container ({e}); skipping"))
        .ok()?;
    let url = format!(
        "mysql://root@127.0.0.1:{}/test",
        node.get_host_port_ipv4(3306).await.unwrap()
    );
    servers._mysql = Some(node);
    Some(url)
}

fn target(id: &str, db: Database, in_tokio: bool, sync: bool) -> Target {
    Target {
        id: id.to_owned(),
        users: Arc::new(Repo::new(db.clone())),
        db,
        in_tokio,
        sync,
    }
}

/// Connects and seeds every async-capable target; returns the Postgres URL
/// too, for `postgres-sync`, which must connect outside the runtime.
async fn targets(servers: &mut Servers) -> (Vec<Target>, Option<String>) {
    let mut out = Vec::new();
    let mut add = |id: &str, db: Database, in_tokio: bool, sync: bool| {
        out.push(target(id, db, in_tokio, sync));
    };
    let postgres = postgres_url(servers).await;

    if let Some(url) = &postgres {
        let pg = connect(url, "postgres").await;
        seed(&pg, POSTGRES_DDL).await;
        add("postgres", pg, true, false);
        add(
            "sqlx-postgres",
            connect(url, "sqlx-postgres").await,
            true,
            false,
        );
    }

    if let Some(url) = mysql_url(servers).await {
        let my = connect(&url, "mysql").await;
        seed(&my, MYSQL_DDL).await;
        add("mysql", my, true, true);
        add("sqlx-mysql", connect(&url, "sqlx-mysql").await, true, false);
    }

    let dir = std::env::temp_dir().join(format!("rupa-bench-drivers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for driver in ["sqlite", "sqlx-sqlite"] {
        let path = dir.join(format!("{driver}.db"));
        let config: DbConfig = serde_json::from_value(serde_json::json!({
            "url": format!("sqlite://{}", path.display()).replace('\\', "/"),
            "driver": driver,
            "journal_mode": "wal",
            "pool": { "max_size": POOL },
        }))
        .unwrap();
        let db = rupa::connect(&config).await.unwrap();
        seed(&db, SQLITE_DDL).await;
        add(driver, db, true, driver == "sqlite");
    }

    (out, postgres)
}

/// Ids cycle through the seeded rows, so caches stay warm but rows differ.
fn id(i: u64) -> i64 {
    (i % ROWS as u64) as i64 + 1
}

fn new_user(i: u64) -> NewUser {
    NewUser {
        email: format!("bench-{i}@example.com"),
        name: None,
        active: true,
        prefs: Prefs {
            theme: "dark".into(),
            beta: false,
        },
    }
}

async fn get_by_id(users: &dyn Users, i: u64) {
    users.by_id(id(i)).await.unwrap().unwrap();
}

async fn active_100(users: &dyn Users) {
    assert_eq!(users.active(100).await.unwrap().len(), 100);
}

async fn insert_rollback(db: &Database, i: u64) {
    let mut tx = db.begin().await.unwrap();
    tx.run(insert::<User>().value(&new_user(i)).affected())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
}

/// Times `iters` runs of `work` inside one task on a runtime worker, as a
/// request handler would run them.
fn in_task<F, Fut>(rt: &Runtime, iters: u64, work: F) -> Duration
where
    F: Fn(u64) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    rt.block_on(rt.spawn(async move {
        let start = Instant::now();
        for i in 0..iters {
            work(i).await;
        }

        start.elapsed()
    }))
    .unwrap()
}

fn bench_one(g: &mut BenchmarkGroup<'_, WallTime>, rt: &Runtime, t: &Target, workload: &str) {
    if t.in_tokio {
        let (db, users) = (t.db.clone(), t.users.clone());
        g.bench_function(format!("async/{}", t.id), |b| {
            b.iter_custom(|iters| {
                let (db, users) = (db.clone(), users.clone());
                match workload {
                    "get_by_id" => in_task(rt, iters, move |i| {
                        let users = users.clone();
                        async move { get_by_id(&*users, i).await }
                    }),
                    "active_100" => in_task(rt, iters, move |_| {
                        let users = users.clone();
                        async move { active_100(&*users).await }
                    }),
                    "insert_rollback" => in_task(rt, iters, move |i| {
                        let db = db.clone();
                        async move { insert_rollback(&db, i).await }
                    }),
                    _ => unreachable!(),
                }
            })
        });
    }

    if t.sync {
        // Sync drivers' futures are already complete; no runtime involved.
        let (db, users) = (&t.db, &*t.users);
        g.bench_function(format!("sync/{}", t.id), |b| {
            let mut i = 0;
            b.iter(|| {
                i += 1;
                pollster::block_on(async {
                    match workload {
                        "get_by_id" => get_by_id(users, i).await,
                        "active_100" => active_100(users).await,
                        "insert_rollback" => insert_rollback(db, i).await,
                        _ => unreachable!(),
                    }
                })
            })
        });
    }
}

fn bench_concurrent(g: &mut BenchmarkGroup<'_, WallTime>, rt: &Runtime, t: &Target) {
    if t.in_tokio {
        let users = t.users.clone();
        g.bench_function(format!("async/{}", t.id), |b| {
            b.iter(|| {
                rt.block_on(async {
                    let tasks: Vec<_> = (0..CONCURRENT)
                        .map(|i| {
                            let users = users.clone();
                            tokio::spawn(async move { get_by_id(&*users, i).await })
                        })
                        .collect();
                    for task in tasks {
                        task.await.unwrap();
                    }
                })
            })
        });
    }

    if t.sync {
        let users = &*t.users;
        // 8 threads, started once per sample (not per iteration, which would
        // mostly measure thread creation), each doing 8 lookups per iteration.
        let per_thread = CONCURRENT / u64::from(POOL);
        g.bench_function(format!("sync/{}", t.id), |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                std::thread::scope(|s| {
                    for thread in 0..u64::from(POOL) {
                        s.spawn(move || {
                            for i in 0..iters * per_thread {
                                pollster::block_on(get_by_id(users, thread * 1_000 + i));
                            }
                        });
                    }
                });
                start.elapsed()
            })
        });
    }
}

fn drivers(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut servers = Servers::default();
    let (mut targets, postgres) = rt.block_on(targets(&mut servers));
    if let Some(url) = postgres {
        // The `postgres` crate runs its own runtime: connect from a plain thread.
        let db = pollster::block_on(connect(&url, "postgres-sync"));
        let t = target("postgres-sync", db, false, true);
        // Checked, not assumed: one lookup from a runtime task.
        let users = t.users.clone();
        let probe = rt.block_on(rt.spawn(async move { get_by_id(&*users, 1).await }));
        eprintln!(
            "postgres-sync inside tokio: {}",
            match probe {
                Ok(()) => "works".to_owned(),
                Err(e) => format!("fails ({e})"),
            }
        );
        targets.insert(2, t);
    }

    eprintln!(
        "drivers: {} (tokio workers: {})",
        targets
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );

    for workload in ["get_by_id", "active_100", "insert_rollback"] {
        let mut g = c.benchmark_group(format!("drivers/{workload}"));
        g.warm_up_time(Duration::from_secs(1));
        g.measurement_time(Duration::from_secs(3));
        for t in &targets {
            bench_one(&mut g, &rt, t, workload);
        }

        g.finish();
    }

    let mut g = c.benchmark_group("drivers/concurrent_64");
    g.throughput(Throughput::Elements(CONCURRENT));
    g.warm_up_time(Duration::from_secs(1));
    g.measurement_time(Duration::from_secs(3));
    for t in &targets {
        bench_concurrent(&mut g, &rt, t);
    }

    g.finish();
    drop(targets);
    // Containers are removed asynchronously, on a runtime.
    rt.block_on(async move { drop(servers) });
}

criterion_group!(benches, drivers);
criterion_main!(benches);
