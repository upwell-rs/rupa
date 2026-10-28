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

#[repository]                           // dyn-compatible: inject Arc<dyn UserRepository>
pub trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, rupa::DynError>;

    #[query(filter = rupa::dsl::ilike(email, $pattern) && prefs.theme == $theme, order_by = id)]
    async fn search(&self, pattern: &str, theme: &str) -> Result<Vec<User>, rupa::DynError>;

    #[query(insert, returning)]
    async fn create(&self, user: &NewUser) -> Result<User, rupa::DynError>;
}

let db = PgPool::connect(&DbConfig::new("postgres://app@localhost/app")).await?;   // a pool
let users = UserRepositoryRepo::new(&db);
```

Everything a statement runs on is a `Connection`: a pool, a single
connection, a transaction, and any `&`, `&mut`, `Box`, `Rc` or `Arc` of one.
Queries run on one with `execute`, and the connection decides whether to
`.await`:

```rust
let open = select::<User>().filter(User::NICKNAME.is_null()).all().execute(&pool)?;       // sync
let open = select::<User>().filter(User::NICKNAME.is_null()).all().execute(&tx).await?;   // async
```

A repository holds one connection, so repositories share a transaction by
borrowing it, and `commit` takes the transaction once they are done:

```rust
let tx = pool.begin().await?;
NoteWritesRepo::new(&tx).add(&note).await?;
NoteReadsRepo::new(&tx).all().await?;
tx.commit().await?;
```

Repository traits are dyn-compatible by default, so they can be injected as
`Arc<dyn Trait>`. Use `#[repository(static_dispatch)]` for native, unboxed
futures when the trait is only used through its generated `Repo` type. Async
repositories run on sync connections too, without an adapter.

Repositories are tested without a database: the in-memory backend evaluates
the same queries with Postgres semantics, so application code that depends on
the trait runs unchanged in a unit test.

```rust
let mut db = rupa::drivers::memory::MemoryDb::new();
db.register::<User>();

let users = UserRepositoryRepo::new(db);
users.create(&new_user).await?;     // the same query, evaluated in memory
```

See [rupa, Diesel and SeaORM](docs/comparison.md) for the same tasks side by
side, with what each does better.

## Examples

See [`docs/architecture.md`](docs/architecture.md) for how a query flows
through the crates. The small examples below use SQLite in memory; they need
no database server, Docker, or database file.

| Example | What it shows | Run |
|---|---|---|
| [SQL/Rust migrations](examples/migrations) | `db` crate with numbered and timestamped migration streams; an `app` crate embedding the operator command line | `cargo run -p rupa-example-app -- primary apply` |
| [Upwell databases](examples/migrations/upwell) | Two `database!` declarations in an Upwell application: field-by-field and templated configuration, startup apply and check, an injected `Arc<dyn NoteRepository>`, migration commands | `cargo run -p rupa-example-upwell -- --config examples/migrations/upwell/config migrate apply` |
| [CRUD and transactions](crates/rupa/examples/sqlite_crud.rs) | Entity capabilities, typed insert and patch values, select, get, rollback, commit, delete | `cargo run -p rupa --example sqlite_crud --features sqlite` |
| [Joins](crates/rupa/examples/sqlite_joins.rs) | Declared relations, inner and left joins, `Joined`, `#[query]`, bound SQL | `cargo run -p rupa --example sqlite_joins --features sqlite` |
| [Repositories](crates/rupa/examples/sqlite_repository.rs) | `#[repository]` with a statically typed SQLite pool | `cargo run -p rupa --example sqlite_repository --features sqlite` |
| [Repository CRUD](crates/rupa/examples/sqlite_repository_crud.rs) | Inserts with and without `returning`, one and many, get, patch, delete and a filtered select through subset shapes | `cargo run -p rupa --example sqlite_repository_crud --features sqlite` |
| [Repository transaction](crates/rupa/examples/sqlite_repository_tx.rs) | Two repositories sharing one transaction by reference, explicit rollback and commit | `cargo run -p rupa --example sqlite_repository_tx --features sqlite` |
| [Testing in memory](crates/rupa/examples/memory_repository_test/main.rs) | A service unit-tested through its repository on `MemoryDb`, no database at all | `cargo test -p rupa --example memory_repository_test --features memory` |
| [DSL and JSON](crates/rupa/examples/sqlite_dsl.rs) | Custom DSL lowering, JSON paths, dialect rendering, bound parameters | `cargo run -p rupa --example sqlite_dsl --features sqlite` |
| [Dynamic connection](crates/rupa/examples/sqlite_config.rs) | `DbConfig`, runtime driver selection, raw DDL, async pooled repository | `cargo run -p rupa --example sqlite_config --features sqlite,dynamic` |
| [Async companion](crates/rupa/examples/sqlite_async.rs) | The same `Task` model and a repository on an async pool and transaction | `cargo run -p rupa --example sqlite_async --features sqlx-sqlite` |

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
| `sync` | sync execution where a mode split exists (so far migrations: `rupa::migrate::sync`); enabled by sync drivers |
| `async` | async executors, transactions, pools and `async fn` repository methods (no runtime by itself) |
| `dynamic` | run-time driver selection and type erasure: `rupa::connect`, `rupa::registry()`, `Database`, boxed executors |
| `tokio` | sync methods of async repositories block with tokio's `block_in_place` |
| `tls-rustls` | TLS on rustls (ring) for every enabled networked driver |
| `migrations` | explicit SQL/Rust migrations; native SQLite, and PostgreSQL on sync and tokio-postgres drivers ([guide](docs/migrations.md)) |
| `migration-commands` | operator commands over the migration engine, with human/JSON reports, exit statuses and redaction ([guide](docs/migrations.md#operator-commands), [runbooks](docs/migrations-runbooks.md)) |
| `migration-cli` | `migration-commands` plus an embeddable clap command line |
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
let tx = db.begin().await?;
UserRepositoryRepo::new(&tx).create(&new_user).await?;
tx.commit().await?;
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
- **In progress:** the Upwell integration (`rupa-upwell`). Declared databases
  with startup migrations, repository providers and migration commands are
  done. Scope-dependent integration waits for Upwell's scopes epic.

Requires Rust 1.99. Licensed under MIT OR Apache-2.0.
