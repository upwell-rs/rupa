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

let users: Arc<dyn UserRepository> = Arc::new(Repo::shared_async(executor));
```

See [`crates/rupa/examples/tour.rs`](crates/rupa/examples/tour.rs) for a
runnable tour (`cargo run -p rupa --example tour`, no database needed).

## Backends

| backend | crate | sync/async |
|---|---|---|
| Postgres | `rupa-driver-postgres`, `rupa-driver-tokio-postgres` | sync, async |
| SQLite (3.38+) | `rupa-driver-rusqlite` | sync |
| MySQL (8.0+) | rendering verified; driver (sqlx) planned | — |
| In memory | `rupa-driver-memory` | sync, async |

Repositories with `async` methods also run on sync executors without an
async runtime.

## Status

Pre-1.0 and under active development. The design is documented milestone by
milestone in [`docs/design`](docs/design).

- **Done:** entities, queries and repositories, transactions with savepoints,
  DSL functions, Postgres/MySQL/SQLite rendering, and native row-level
  security.
- **Next:** the Upwell dependency-injection integration (`rupa-upwell`) and
  the remaining drivers.

Requires Rust 1.99. Licensed under MIT OR Apache-2.0.
