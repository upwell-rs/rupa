# rupa

## Role

The facade: the crate applications depend on. It re-exports the sans-IO core,
the derive and attribute macros, the SQL renderer and the built-in DSL
functions, and puts each driver behind a cargo feature.

## Usage

```toml
[dependencies]
rupa = { version = "0.1", features = ["sqlite"] }            # statically typed, sync
rupa = { version = "0.1", features = ["postgres"] }          # async (tokio-postgres)
rupa = { version = "0.1", features = ["full"] }              # every driver + run-time selection
```

```rust
use rupa::prelude::*;

let pool = rupa::drivers::sqlite::SqlitePool::connect(&DbConfig::new("sqlite://app.db"))?;
let users = UsersView::new(&pool);
```

For runnable, server-free examples, start with the [SQLite example guide](../../README.md#examples).

## Internal role

Owns the feature graph: every facade feature forwards to the matching
features of the core, macro and driver crates. With `dynamic`, it also builds
`rupa::registry()` from the enabled drivers. `__macro_support` holds the
paths macro output refers to.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date, time and UUID columns, forwarded to every enabled driver |
| `async` | async executors, transactions, pools and `async fn` repository methods |
| `tokio` | sync methods of async repositories block with `block_in_place` |
| `dynamic` | run-time driver selection and type erasure: `registry()`, `connect`, `Database`, boxed executors (implies `async`) |
| `postgres` | async Postgres on tokio-postgres + deadpool (implies `async`) |
| `postgres-sync` | sync Postgres on `postgres` + r2d2 |
| `sqlite` / `sqlite-unbundled` | SQLite on rusqlite + r2d2, compiled in / linked from the system |
| `mysql` | sync MySQL on `mysql` + r2d2 |
| `memory` | the in-memory backend |
| `sqlx-postgres`, `sqlx-mysql`, `sqlx-sqlite`, `sqlx-sqlite-unbundled` | async drivers on sqlx + deadpool (imply `async`) |
| `tls-rustls` | TLS on rustls (ring) for every enabled networked driver |
| `migrations` | explicit SQL/Rust migrations, applied on the enabled native SQLite, PostgreSQL and MySQL drivers ([guide](../../docs/migrations.md)) |
| `full` | every driver, plus `dynamic` and `migrations` |
| `upwell` | the Upwell DI integration (in development) |
