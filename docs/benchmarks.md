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
| `build_render` | building queries; rendering them for Postgres, MySQL and SQLite | nothing |
| `execute` | in-memory and SQLite round trips through each layer; pool checkout and owned transactions | nothing (SQLite runs in memory) |
| `postgres` | real round trips: rupa vs the raw tokio-postgres client vs a repository over `Database` | `RUPA_BENCH_POSTGRES_URL` (skipped without it) |

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

What they say:

- **Rendering is the biggest fixed cost** in a cached SQLite lookup (~350 ns
  of ~2 µs). Caching rendered SQL per query site is the next optimization.
- **Repositories are nearly free** (+60 ns), and a pooled, erased `Database`
  adds ~0.6 µs per call (checkout, boxing, dynamic dispatch). That is noise
  next to a network round trip.
- **The memory backend is a test backend.** Lookups scan, and transactions
  snapshot every table.
