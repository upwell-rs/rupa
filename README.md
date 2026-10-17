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

#[repository]                           // dyn-compatible: Arc<dyn UserRepository>
pub trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, rupa::DynError>;

    #[query(filter = rupa::dsl::ilike(email, $pattern) && prefs.theme == $theme, order_by = id)]
    async fn search(&self, pattern: &str, theme: &str) -> Result<Vec<User>, rupa::DynError>;

    #[query(insert, returning)]
    async fn create(&self, user: &NewUser) -> Result<User, rupa::DynError>;
}

let db = rupa::connect(&DbConfig::new("postgres://app@localhost/app")).await?;   // a pool
let users: Arc<dyn UserRepository> = Arc::new(Repo::new(db));
```

See [`crates/rupa/examples/tour.rs`](crates/rupa/examples/tour.rs) for a
runnable tour (`cargo run -p rupa --example tour`, no database needed).

## Backends

Enable drivers with cargo features of `rupa`; each one is registered in
`rupa::registry()`, so a configuration can pick it at run time.

| backend | feature | driver | sync/async | pool |
|---|---|---|---|---|
| Postgres | `postgres` | `rupa-driver-tokio-postgres` | async | deadpool |
| Postgres | `postgres-sync` | `rupa-driver-postgres` | sync | r2d2 |
| SQLite (3.38+) | `sqlite` | `rupa-driver-rusqlite` | sync | r2d2 |
| MySQL (8.0+) | `mysql` | `rupa-driver-mysql` (sqlx planned) | sync | r2d2 |
| In memory | `memory` | `rupa-driver-memory` | sync, async | one shared connection |

Repositories with `async` methods also run on sync executors without an
async runtime.

## Connecting from configuration

A `DbConfig` holds the shared settings plus the driver's typed options.
It is serde-friendly, so it can live in your configuration file:

```toml
[database]
url = "sqlite://data/app.db"   # the scheme picks the driver...
driver = "sqlite"              # ...or name it: postgres, postgres-sync, sqlite, mysql, memory
busy_timeout = "2s"            # options of that driver, checked against its type
journal_mode = "wal"

[database.pool]
max_size = 8
acquire_timeout = "5s"
```

```rust
let db: rupa::Database = rupa::connect(&config.database).await?;
let mut tx = db.begin().await?;                  // owns its connection: 'static
Repo::new(&mut tx).rename(id, "x").await?;
tx.commit().await?;
```

`Database` is a cheaply cloneable, type-erased pool. Each driver's pool
(`SqlitePool`, `PgPool`, ..) can also be used directly, statically typed. See
[`docs/design/connections.md`](docs/design/connections.md).

## Benchmarks

`cargo bench -p rupa-bench`; see [`docs/benchmarks.md`](docs/benchmarks.md).

## Status

Pre-1.0 and under active development. The design is documented milestone by
milestone in [`docs/design`](docs/design).

- **Done:** entities, queries and repositories, transactions with savepoints,
  DSL functions, Postgres/MySQL/SQLite rendering and drivers, native
  row-level security, connection pools, configuration-driven drivers, and
  caching of rendered SQL and prepared statements.
- **Next:** the Upwell dependency-injection integration (`rupa-upwell`) and
  async drivers on sqlx.

Requires Rust 1.99. Licensed under MIT OR Apache-2.0.
