# Benchmarks

Criterion benchmarks live in [`crates/rupa-bench`](../crates/rupa-bench)
(not published). They measure rupa's own cost: building and rendering
queries, the executor and repository layers, pools, and type erasure.

```sh
cargo bench -p rupa-bench                               # everything except Postgres
cargo bench -p rupa-bench --bench execute -- sqlite     # one group (a regex filter)
cargo bench -p rupa-bench -- --quick                    # fast and rough
```

## Suites

| bench | measures | needs |
|---|---|---|
| `build_render` | building queries; rendering them for Postgres, MySQL and SQLite, uncached (`render`) and through the render cache (`render_cached`) | nothing |
| `execute` | in-memory and SQLite round trips through each layer; pool checkout and owned transactions | nothing (SQLite runs in memory) |
| `postgres` | real round trips: rupa vs the raw tokio-postgres client vs a repository over `Database` | `RUPA_BENCH_POSTGRES_URL` (skipped without it) |
| `drivers` | every driver on the same workloads, in a tokio task (`async/..`) and from plain threads (`sync/..`) | Docker (testcontainers), or `RUPA_BENCH_POSTGRES_URL` / `RUPA_BENCH_MYSQL_URL` |

The `execute` and `postgres` groups compare one lookup through each layer,
so the difference between two rows is the overhead of one layer:

- `executor`: the driver's executor, statically dispatched;
- `repo_shared`: an `Arc<dyn Users>` repository over `Repo::shared`;
- `repo_database`: the same repository over a pooled, type-erased
  `Database` (what `rupa::connect` returns);
- `raw_client` (Postgres only): tokio-postgres with a prepared statement.

## Tracking changes

Criterion compares each run with the previous one on the same machine. To
compare a change against `master`:

```sh
git checkout master && cargo bench -p rupa-bench -- --save-baseline master
git checkout my-branch && cargo bench -p rupa-bench -- --baseline master
```

CI runs `build_render` and `execute` in `--quick` mode on every push to
`master` and keeps `target/criterion` as a build artifact. Shared runners
are noisy, so that history catches large regressions; judge small ones
locally against a baseline.

## First results

2026-10-03, `--quick`, Windows 11, Rust 1.99, release profile. Medians:

| benchmark | time |
|---|---|
| build `get_by_id` / `select_filtered_page` / `insert_100` | 175 ns / 556 ns / 40.7 µs |
| render `get_by_id` (any dialect) | ~340 ns |
| render `select_filtered_page` (Postgres / SQLite / MySQL) | 0.87 / 1.17 / 1.45 µs |
| render `insert_100` | 30–35 µs |
| SQLite `get_by_id`: `executor` / `repo_shared` / `repo_database` | 2.05 / 2.11 / 2.67 µs |
| SQLite insert in a rolled-back transaction | 4.5 µs |
| pool checkout + return: `SqlitePool` / `Database` | 211 / 299 ns |
| pool `begin` + `rollback`: `SqlitePool` / `Database` | 0.85 / 1.04 µs |
| memory `get_by_id` (1000 rows, full scan) | 39.8 µs |
| memory insert in a rolled-back transaction | 942 µs |

What they said, at the time:

- **Rendering was the biggest fixed cost** in a cached SQLite lookup (~350 ns
  of ~2 µs). Since cached by shape; see "Render cache" below.
- **Repositories are nearly free** (+60 ns), and a pooled, erased `Database`
  adds ~0.6 µs per call (checkout, boxing, dynamic dispatch). That is noise
  next to a network round trip.
- **The memory backend is a test backend.** Lookups scan, and transactions
  snapshot every table.

## Render cache

2026-10-17, `--quick`, same machine and session (this day's machine was ~20%
slower across the board than the first results, so compare within the
table):

| benchmark | `render` | `render_cached` |
|---|---|---|
| `get_by_id`, Postgres / MySQL / SQLite | 387 / 452 / 417 ns | 190 / 164 / 163 ns |
| `select_filtered_page`, Postgres / MySQL / SQLite | 1.09 / 1.80 / 1.49 µs | 342 / 340 / 348 ns |
| `insert_100` | 36–50 µs | 33–35 µs |
| SQLite `get_by_id`, end to end | 2.33 µs (`executor_render_uncached`) | 2.18 µs (`executor`) |

See [`design/caching.md`](design/caching.md) for what is cached and why
values never are.


## Drivers

2026-10-17, Windows 11, 12 tokio workers, pool of 8, all through
`Database` + an `Arc<dyn Users>` repository. Postgres 17 and MySQL 8 run in
Docker Desktop. Medians from two runs:

| workload | postgres | sqlx-postgres | postgres-sync¹ | mysql (in tokio / sync) | sqlx-mysql | sqlite (in tokio / sync) | sqlx-sqlite |
|---|---|---|---|---|---|---|---|
| `get_by_id` | 391–398 µs | 447 µs² | 494–542 µs | 394–603 / 459–649 µs | 547–633 µs | 6.1–8.3 / 5.7–6.4 µs | 26.5–37.3 µs |
| `active_100` | 542 µs | 570 µs | 605 µs | 653 / 604 µs | 638 µs | 115 / 123 µs | 282 µs |
| `insert_rollback` | 1.13 ms | 1.03 ms | 1.14 ms | 6.83 / 6.91 ms³ | 6.89 ms | 18.6 / 18.7 µs | 108 µs |
| `concurrent_64` (lookups/s) | 9.8–10.5 K | 6.6–10.8 K | 3.0 K⁴ | 2.5–9.8 K / 6.2 K | 9.1–9.7 K | 290–354 K / 609 K | 152–185 K |

1. Sync only: the `postgres` crate drives its own runtime and panics inside
   tokio's ("Cannot start a runtime from within a runtime"), so it can't
   serve async code at all.
2. The first run measured 1.0 ms with a wide interval; the rerun 447 µs.
3. The same for every MySQL driver, so it's the server (InnoDB undo on
   rollback), not the driver.
4. 8 OS threads × 8 lookups, threads created once per sample.

What the numbers support:

- **Networked databases: the round trip dominates.** Through Docker
  Desktop it costs ~400 µs, and results vary 2–4× between runs under
  concurrency. Driver differences (a few to tens of µs of CPU) are below
  that noise, so these runs can't rank tokio-postgres against sqlx-postgres,
  or mysql against sqlx-mysql, on speed. Comparing those needs a native
  server on Linux.
- **SQLite: rusqlite is 3–6× faster than sqlx-sqlite per statement.** That
  holds for lookups, decoding 100 rows and writes. sqlx hands every
  statement to a per-connection worker thread and back, which costs
  ~20–30 µs. It also wins on concurrent throughput, even through `Blocking`
  inside tokio.
- **Sync drivers inside tokio block a worker for the whole statement.**
  For SQLite that is ~6 µs and doesn't show. For a networked database it is
  the round trip, and with more concurrent requests than workers, tasks wait
  behind blocked workers. That likely explains the wide `concurrent_64`
  spread for `mysql` in tokio. The benchmark doesn't measure the delay this
  causes other tasks.

## Cleanup pass

2026-10-18, `--quick`, macOS (Apple silicon), Rust 1.99, the same session
before and after. The pass swapped the hand-rolled LRU for `hashlink`, split
the core into sync / `async` / `dynamic` modules, made rusqlite bind text and
blobs without copying, and moved shared pool and error code into macros.
Criterion reported no regression (34 benchmarks unchanged within noise, one
improved):

| benchmark | before | after |
|---|---|---|
| render `get_by_id`, Postgres / MySQL / SQLite | 271 / 262 / 258 ns | 270 / 264 / 260 ns |
| render_cached `get_by_id`, Postgres / MySQL / SQLite | 135 / 128 / 129 ns | 135 / 128 / 126 ns |
| render_cached `select_filtered_page` (any dialect) | 266–277 ns | 268–271 ns |
| render vs render_cached `insert_100` | 20–26 µs / 21 µs | 21–26 µs / 21 µs |
| SQLite `get_by_id`: `executor` / `repo_shared` / `repo_database` | 1.31 / 1.37 / 1.62 µs | 1.33 / 1.41 / 1.65 µs |
| pool checkout + return: `SqlitePool` / `Database` | 154 / 200 ns | 143 / 197 ns |

What they say:

- **The render cache pays off** for typical statements: 2× for a lookup and
  2.6–4× for a filtered page. It gains nothing for large multi-row inserts:
  shaping clones every bound value, which costs as much as rendering. Binding
  borrowed values on a hit is the next step there.
- **The `hashlink` LRU is as fast as the custom one** on the hit path (the
  lookup is a hash and a pointer relink either way).
- **Static dispatch is the cheap path.** A repository over a typed pool adds
  ~70 ns to a SQLite lookup; the erased `Database` (feature `dynamic`) adds
  another ~0.25 µs.
