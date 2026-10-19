# RUPA: Rust Unified Persistence API

An explicit, attribute-driven repository layer for Rust, over any SQL database
and any driver.

- **Nothing is inferred from names.** Attributes and types are the whole
  specification.
- **Building a query never does IO.** Queries are plain values (`Query<R>`),
  rendered per dialect or evaluated in memory.
- **Values are always bound parameters**, including inside your own DSL
  functions.
- **Nothing is generated that you didn't opt into.** An entity has no
  insert, update or delete surface until you derive it.
- **A query means the same thing on every backend.** Where engines differ,
  RUPA writes SQL that reproduces Postgres' semantics. A shared conformance
  suite checks this on every backend.

```rust
use rupa::prelude::*;

#[derive(Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "users", schema = "app")]
struct User {
    #[id(generated)] id: i64,
    email: String,
    nickname: Option<String>,
    prefs: Prefs,                       // Serialize + Deserialize: a JSON column
}

#[derive(Insertable)]
#[insertable(entity = User)]
struct NewUser { email: String, prefs: Prefs }

#[repository]                           // native, unboxed futures
pub trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, rupa::DynError>;

    #[query(filter = rupa::dsl::ilike(email, $pattern) && prefs.theme == $theme, order_by = id)]
    async fn search(&self, pattern: &str, theme: &str) -> Result<Vec<User>, rupa::DynError>;

    #[query(insert, returning)]
    async fn create(&self, user: &NewUser) -> Result<User, rupa::DynError>;
}

let db = PgPool::connect(&DbConfig::new("postgres://app@localhost/app")).await?;   // a pool
let users = UserRepositoryView::new(&db);
```

Async repository futures are unboxed by default. Use
`#[repository(dynamic_dispatch)]` when a trait must support `dyn Trait`.
Repositories open native transactions with `begin` and `begin_with_security`;
exclusive `&mut self` queries borrow those transactions directly. Shared
`&self` transaction views use an explicit `TxSession` or `AsyncTxSession`.

## Examples

See [`docs/architecture.md`](docs/architecture.md) for how a query flows
through the crates. The small examples below use SQLite in memory; they need
no database server, Docker, or database file.

| Example | What it shows | Run |
|---|---|---|
| [SQL/Rust migrations](crates/rupa/examples/sqlite_migrations.rs) | Immutable history, typed Rust data steps, per-migration transactions | `cargo run -p rupa --example sqlite_migrations --features migrations,sqlite` |
| [CRUD and transactions](crates/rupa/examples/sqlite_crud.rs) | Entity capabilities, typed insert and patch values, select, get, rollback, commit, delete | `cargo run -p rupa --example sqlite_crud --features sqlite` |
| [Joins](crates/rupa/examples/sqlite_joins.rs) | Declared relations, inner and left joins, `Joined`, `#[query]`, bound SQL | `cargo run -p rupa --example sqlite_joins --features sqlite` |
| [Repositories](crates/rupa/examples/sqlite_repository.rs) | `#[repository]` with a statically typed SQLite pool | `cargo run -p rupa --example sqlite_repository --features sqlite` |
| [Repository transaction](crates/rupa/examples/sqlite_repository_tx.rs) | Two repository views sharing one transaction, explicit rollback and commit | `cargo run -p rupa --example sqlite_repository_tx --features sqlite` |
| [DSL and JSON](crates/rupa/examples/sqlite_dsl.rs) | Custom DSL lowering, JSON paths, dialect rendering, bound parameters | `cargo run -p rupa --example sqlite_dsl --features sqlite` |
| [Dynamic connection](crates/rupa/examples/sqlite_config.rs) | `DbConfig`, runtime driver selection, raw DDL, async pooled repository | `cargo run -p rupa --example sqlite_config --features sqlite,dynamic` |
| [Async companion](crates/rupa/examples/sqlite_async.rs) | The same `Task` and repository view API on an async transaction | `cargo run -p rupa --example sqlite_async --features sqlx-sqlite` |

The main examples use synchronous SQLite. The async companion shows API parity
with a native async driver; for SQLite workloads, the synchronous driver is
the faster default.

Native row-level security needs Postgres. Its end-to-end coverage lives in the
[sync](crates/rupa-driver-postgres/tests/rls.rs) and
[async](crates/rupa-driver-tokio-postgres/tests/rls.rs) testcontainers tests.

## Features

Nothing is compiled that you didn't ask for. The default build is the sync,
statically typed core with `chrono` and `uuid` column types; everything else
is a cargo feature of `rupa`:

| feature | adds |
|---|---|
| `async` | async executors, transactions, pools and `async fn` repository methods (no runtime by itself) |
| `dynamic` | run-time driver selection and type erasure: `rupa::connect`, `rupa::registry()`, `Database`, boxed executors |
| `tokio` | sync methods of async repositories block with tokio's `block_in_place` |
| `tls-rustls` | TLS on rustls (ring) for every enabled networked driver |
| `migrations` | explicit SQL/Rust migrations; native SQLite apply ([guide](docs/migrations.md)) |
| `full` | every driver, plus `dynamic` and `migrations` |

Most applications know their database: use its pool directly
(`rupa::drivers::sqlite::SqlitePool`, `rupa::drivers::postgres::PgPool`), which
is statically dispatched. `dynamic` is for the cases that pick a database
from configuration at run time.

## Backends

Each driver is a cargo feature of `rupa`, re-exported under `rupa::drivers`.
With `dynamic`, it is also registered in `rupa::registry()`, so a
configuration can pick it at run time.

| backend | feature | driver | sync/async | pool |
|---|---|---|---|---|
| Postgres | `postgres` | `rupa-driver-tokio-postgres` | async | deadpool |
| Postgres | `sqlx-postgres` | `rupa-driver-sqlx` | async | deadpool |
| Postgres | `postgres-sync` | `rupa-driver-postgres` | sync (not inside tokio) | r2d2 |
| SQLite (3.38+) | `sqlite` (`sqlite-unbundled`: system SQLite) | `rupa-driver-rusqlite` | sync | r2d2 |
| SQLite (3.38+) | `sqlx-sqlite` (`sqlx-sqlite-unbundled`) | `rupa-driver-sqlx` | async (worker thread per connection) | deadpool |
| MySQL (8.0+) | `sqlx-mysql` | `rupa-driver-sqlx` | async | deadpool |
| MySQL (8.0+) | `mysql` | `rupa-driver-mysql` | sync | r2d2 |
| In memory | `memory` | `rupa-driver-memory` | sync, async | one shared connection |

When two drivers serve a URL scheme, the one listed first wins unless the
configuration names a driver (`driver = "mysql"`). Networked databases
prefer async drivers; SQLite prefers rusqlite, which is 3–6× faster per
statement (see [`docs/benchmarks.md`](docs/benchmarks.md), "Drivers"). A configuration that
names a driver that isn't compiled in is served by another one for the same
database. TLS: `tls-rustls` (native-tls is not offered, as it pulls in
OpenSSL on Linux).

Repositories with `async` methods also run on sync executors without an
async runtime.

## Connecting from configuration

A `DbConfig` holds the shared settings plus the driver's typed options.
It is serde-friendly, so it can live in your configuration file:

```toml
[database]
url = "sqlite://data/app.db"   # the scheme picks the driver...
driver = "sqlite"              # ...or name it: postgres, postgres-sync, sqlite, mysql, memory, sqlx-*
busy_timeout = "2s"            # options of that driver, checked against its type
journal_mode = "wal"

[database.pool]
max_size = 8
acquire_timeout = "5s"
```

```rust
let db: rupa::Database = rupa::connect(&config.database).await?;   // `dynamic`
let tx = UserRepositoryView::new(&db).begin().await?;
let session = rupa::AsyncTxSession::new(tx);
UserRepositoryView::new(&session).create(&new_user).await?;
session.commit().await?;
```

`Database` (feature `dynamic`) is a cheaply cloneable, type-erased pool.
Each driver's pool (`SqlitePool`, `PgPool`, ..) takes the same `DbConfig`
and is used directly, statically typed: `SqlitePool::connect(&config)`. See
[`docs/design/connections.md`](docs/design/connections.md).

## Tests

```sh
cargo nextest run --workspace --features rupa/full   # needs Docker for Postgres and MySQL
cargo test --doc --workspace --features rupa/full     # doctests (nextest skips them)
```

Postgres and MySQL suites start their own containers with testcontainers;
[`.config/nextest.toml`](.config/nextest.toml) runs at most two of them at
once. Plain `cargo test` works too, running one test binary at a time.

## Benchmarks

`cargo bench -p rupa-bench`; see [`docs/benchmarks.md`](docs/benchmarks.md).

## Status

Pre-1.0 and under active development. The design is documented milestone by
milestone in [`docs/design`](docs/design).

- **Done:** entities, queries and repositories, transactions with savepoints,
  DSL functions, Postgres/MySQL/SQLite rendering and drivers (sync, and
  async on sqlx), native
  row-level security, connection pools, configuration-driven drivers, and
  caching of rendered SQL and prepared statements.
- **Next:** the Upwell dependency-injection integration (`rupa-upwell`).

Requires Rust 1.99. Licensed under MIT OR Apache-2.0.
