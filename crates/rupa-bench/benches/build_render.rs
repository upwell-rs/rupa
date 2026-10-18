//! The sans-IO half: building queries and rendering them per dialect.
//!
//! Every execution pays for both (rendering isn't cached yet), so these set
//! the floor under every driver benchmark.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rupa::core::{MySql, Postgres, Sqlite};
use rupa::dsl::Dialect;
use rupa::prelude::*;
use rupa_bench::{User, new_users};

fn select_page() -> Query<Vec<User>> {
    select::<User>()
        .filter(
            col!(User::active)
                .eq(true)
                .and(col!(User::email).like("user1%"))
                .and(col!(User::prefs).path("theme").cast::<String>().eq("dark")),
        )
        .order_by(col!(User::id).asc())
        .limit(50)
        .all()
}

fn build(c: &mut Criterion) {
    let mut g = c.benchmark_group("build");
    g.bench_function("get_by_id", |b| b.iter(|| get::<User>(black_box(&42))));
    g.bench_function("select_filtered_page", |b| {
        b.iter(|| black_box(select_page()))
    });
    let one = new_users(1);
    g.bench_function("insert_1", |b| {
        b.iter(|| insert::<User>().values(black_box(&one)).affected())
    });
    let hundred = new_users(100);
    g.bench_function("insert_100", |b| {
        b.iter(|| insert::<User>().values(black_box(&hundred)).affected())
    });
    g.finish();
}

fn render(c: &mut Criterion) {
    let dialects: [(&str, &dyn Dialect); 3] = [
        ("postgres", &Postgres),
        ("mysql", &MySql),
        ("sqlite", &Sqlite),
    ];
    let get = get::<User>(&42);
    let page = select_page();
    let insert_100 = insert::<User>().values(&new_users(100)).affected();

    let mut g = c.benchmark_group("render");
    for (name, dialect) in dialects {
        g.bench_function(format!("get_by_id/{name}"), |b| {
            b.iter(|| rupa::sql::render_query(black_box(&get), dialect).unwrap())
        });
        g.bench_function(format!("select_filtered_page/{name}"), |b| {
            b.iter(|| rupa::sql::render_query(black_box(&page), dialect).unwrap())
        });
        g.bench_function(format!("insert_100/{name}"), |b| {
            b.iter(|| rupa::sql::render_query(black_box(&insert_100), dialect).unwrap())
        });
    }

    g.finish();

    // The same statements through the render cache: every iteration after
    // the first is a hit (shape, lookup, binding the new values).
    let cache = rupa::sql::RenderCache::default();
    let mut g = c.benchmark_group("render_cached");
    for (name, dialect) in dialects {
        g.bench_function(format!("get_by_id/{name}"), |b| {
            b.iter(|| cache.render(black_box(get.statement()), dialect).unwrap())
        });
        g.bench_function(format!("select_filtered_page/{name}"), |b| {
            b.iter(|| cache.render(black_box(page.statement()), dialect).unwrap())
        });
        g.bench_function(format!("insert_100/{name}"), |b| {
            b.iter(|| {
                cache
                    .render(black_box(insert_100.statement()), dialect)
                    .unwrap()
            })
        });
    }

    g.finish();
}

criterion_group!(benches, build, render);
criterion_main!(benches);
