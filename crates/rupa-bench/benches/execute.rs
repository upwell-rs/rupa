//! Executing queries without a server: the in-memory backend and SQLite
//! (in memory, so no disk IO).
//!
//! The SQLite groups compare the same lookup through each layer an
//! application might use, so the difference between them is that layer's
//! overhead:
//!
//! - `executor`: a `SqliteExecutor`, statically dispatched;
//! - `repo_shared`: an `Arc<dyn Users>` repository over `UsersRepo::shared`;
//! - `repo_database`: the same repository over a type-erased, pooled
//!   `Database` (what `rupa::connect` returns).

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rupa::core::exec::{Blocking, Executor};
use rupa::prelude::*;
use rupa::{Database, Pool as _, PoolConfig};
use rupa_bench::{NewUser, SQLITE_DDL, User, Users, UsersRepo, new_users};
use rupa_driver_memory::MemoryDb;
use rupa_driver_rusqlite::{SqliteExecutor, SqliteManager, SqlitePool, SqliteTarget};

const ROWS: usize = 1_000;

fn seeded_sqlite() -> SqliteExecutor {
    let mut exec = SqliteExecutor::open_in_memory().unwrap();
    seed(&mut exec);
    exec
}

fn seed(exec: &mut SqliteExecutor) {
    exec.connection().execute_batch(SQLITE_DDL).unwrap();
    let users = new_users(ROWS);
    for chunk in users.chunks(100) {
        exec.run(insert::<User>().values(chunk).affected()).unwrap();
    }
}

fn seeded_pool() -> SqlitePool {
    let manager = SqliteManager::new(SqliteTarget::Memory);
    let pool = SqlitePool::new(manager, &PoolConfig::default()).unwrap();
    seed(&mut pool.get().unwrap());
    pool
}

/// Ids spread over the table, so lookups don't hit one hot row.
fn ids() -> impl FnMut() -> i64 {
    let mut i = 0;
    move || {
        i = (i + 7919) % ROWS as i64;
        i + 1
    }
}

fn sqlite(c: &mut Criterion) {
    let mut g = c.benchmark_group("sqlite");

    let mut exec = seeded_sqlite();
    let mut next = ids();
    g.bench_function("get_by_id/executor", |b| {
        b.iter(|| exec.run(get::<User>(&next())).unwrap().unwrap())
    });
    g.bench_function("active_page_50/executor", |b| {
        b.iter(|| {
            let q = select::<User>()
                .filter(col!(User::active).eq(true))
                .order_by(col!(User::id).asc())
                .limit(50)
                .all();
            exec.run(q).unwrap()
        })
    });
    let user = NewUser {
        email: "bench@example.com".into(),
        ..new_users(1).remove(0)
    };
    g.bench_function("insert_rolled_back/executor", |b| {
        b.iter(|| {
            let mut tx = exec.begin().unwrap();
            tx.run(insert::<User>().value(black_box(&user)).affected())
                .unwrap();
            tx.rollback().unwrap();
        })
    });

    // The same lookup with the render cache off: the difference is what
    // caching rendered SQL saves per query.
    rupa::sql::render_cache().set_capacity(0);
    g.bench_function("get_by_id/executor_render_uncached", |b| {
        b.iter(|| exec.run(get::<User>(&next())).unwrap().unwrap())
    });
    rupa::sql::render_cache().set_capacity(rupa::sql::cache::DEFAULT_CAPACITY);

    let shared: Arc<dyn Users> = Arc::new(UsersRepo::shared(seeded_sqlite()));
    let mut next = ids();
    g.bench_function("get_by_id/repo_shared", |b| {
        b.iter(|| pollster::block_on(shared.by_id(next())).unwrap().unwrap())
    });

    let pooled: Arc<dyn Users> = Arc::new(UsersRepo::new(Database::new(Blocking(seeded_pool()))));
    let mut next = ids();
    g.bench_function("get_by_id/repo_database", |b| {
        b.iter(|| pollster::block_on(pooled.by_id(next())).unwrap().unwrap())
    });
    g.finish();
}

fn pool(c: &mut Criterion) {
    let mut g = c.benchmark_group("pool");
    let pool = seeded_pool();
    g.bench_function("sqlite_get_and_return", |b| {
        b.iter(|| drop(pool.get().unwrap()))
    });
    g.bench_function("sqlite_begin_rollback", |b| {
        b.iter(|| pool.begin().unwrap().rollback().unwrap())
    });

    let db = Database::new(Blocking(seeded_pool()));
    g.bench_function("database_get_and_return", |b| {
        b.iter(|| drop(pollster::block_on(db.get()).unwrap()))
    });
    g.bench_function("database_begin_rollback", |b| {
        b.iter(|| pollster::block_on(async { db.begin().await.unwrap().rollback().await.unwrap() }))
    });
    g.finish();
}

fn memory(c: &mut Criterion) {
    let mut g = c.benchmark_group("memory");
    let mut db = MemoryDb::new();
    db.register::<User>();
    db.run(insert::<User>().values(&new_users(ROWS)).affected())
        .unwrap();
    let mut next = ids();
    g.bench_function("get_by_id", |b| {
        b.iter(|| db.run(get::<User>(&next())).unwrap().unwrap())
    });
    g.bench_function("insert_rolled_back", |b| {
        b.iter_batched(
            || new_users(1),
            |users| {
                let mut tx = db.begin().unwrap();
                let user = NewUser {
                    email: "bench@example.com".into(),
                    ..users.into_iter().next().unwrap()
                };
                tx.run(insert::<User>().value(&user).affected()).unwrap();
                tx.rollback().unwrap();
            },
            BatchSize::SmallInput,
        )
    });
    let async_db = db.clone().into_async();
    let erased: Arc<dyn Users> = Arc::new(UsersRepo::new(Database::single(async_db)));
    let mut next = ids();
    g.bench_function("get_by_id/repo_database", |b| {
        b.iter(|| pollster::block_on(erased.by_id(next())).unwrap().unwrap())
    });
    g.finish();
}

criterion_group!(benches, sqlite, pool, memory);
criterion_main!(benches);
