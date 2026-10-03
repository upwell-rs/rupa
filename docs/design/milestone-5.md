# Milestone 5: `#[query]` and `#[repository]`

Status: **implemented.**

## `#[query]`: the grammar

```rust
#[query(filter = active == $active && prefs.theme == $theme, order_by = created_at desc, limit = 50)]
fn active_by_theme(active: bool, theme: &str) -> Vec<User>;   // => fn(..) -> Query<Vec<User>>

#[query(sql = "SELECT .. FROM app.users WHERE email = $email")]
fn raw_by_email(email: &str) -> Vec<User>;
```

- **Parser:** a Pratt parser over `syn` tokens. It supports:
  - `== != < <= > >=`, which don't chain;
  - `&& || !`;
  - `in`, `like` and postfix `is_null`;
  - parentheses, `$param`, literals;
  - field paths, where `prefs.theme` is a JSON path;
  - calls (`path(args)`), parsed but rejected until milestone 6.
- **Keys:** `filter`, `order_by` (repeatable; `field [asc|desc]`), `limit` and
  `offset` (a literal or `$param`), `sql`, and `entity` (needed for `bool`
  results). `sql` cannot be combined with the other query keys.
- **Typing comes from the signature, not guesses:**
  - Comparisons go through the typed builder (`ExprOps`), so a parameter's
    Rust type is checked against the column at compile time.
  - A JSON path reads as text for a text parameter and as a cast otherwise:
    `JsonPath::compare<B>` takes `B` from `IntoExpr<B>`.
  - `$x < id` is normalised to `"id" > $x`.
  - A boolean column alone is a condition (`IntoCondition`).
  - Every parameter must be used.
- **Results:**
  - `Vec<T>`, `Option<T>`, `T`: rows of entity `T`;
  - `bool`: `EXISTS`, which needs `entity = T`;
  - raw SQL can also return `u64` or `bool` for affected rows.
- **Raw SQL:** `$name` placeholders are rendered per dialect. A `$` inside
  quotes or comments is ignored; `$1` and dollar-quoting are rejected.
- **Spans:** generated paths are re-spanned onto the user's tokens, so errors
  point at `$n` or at the field, not at the whole attribute.

## `#[repository]`

The author writes an ordinary trait. The macro emits it as written (with
`#[query]` removed and `async fn`s rewritten as described below), plus one
implementation for `Repo<S>`.

```rust
#[repository]                                  // dyn-compatible by default
pub trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, DynError>;

    #[query(filter = active == $active, order_by = id)]
    fn active(&self, active: bool) -> Result<Vec<User>, DynError>;   // blocks in place
}
let users: Arc<dyn UserRepository> = Arc::new(Repo::shared_async(executor));
```

Decided with you on 2026-10-03:

- **The author chooses the receivers.**
  - All `&self`: `S` is a connection source (`Acquire` / `AcquireAsync`).
  - All `&mut self`: `S` is the executor itself, e.g. `Repo::new(&mut tx)`.
  - Mixing the two is a compile error.
- **`async` is chosen per method.** If any query method is `async`, the
  repository runs on an `AsyncExecutor`, and its plain `fn` methods block in
  place.
  - With the facade's `tokio` feature on a multi-threaded runtime, blocking
    uses `tokio::task::block_in_place`.
  - Otherwise it uses `pollster`. That deadlocks if called from a
    current-thread tokio runtime driving the same connection, so enable
    `tokio` when using tokio.
- **Dyn-compatible by default, because Upwell holds `Arc<dyn Trait>`.**
  - `async fn` becomes `fn(..) -> Pin<Box<dyn Future + Send + '_>>`.
  - `#[repository(static_dispatch)]` gives `impl Future + Send` instead:
    zero-cost, but not dyn-compatible.
  - The query is built (sans-IO) before the future starts, so the future
    borrows only `&self`. The parameters don't need extra lifetimes.
- **Errors:** methods return `Result<R, E>`, and the implementation requires
  `E: From<executor error>`. For DI, `rupa::DynError` works with every driver:
  each driver provides `From<DriverError> for DynError`, and the original
  `ResultError` stays inspectable.
- **Hand-written implementors** (mocks, decorators) implement the emitted
  trait directly and are interchangeable behind `Arc<dyn Trait>`.

### Supporting types (`rupa_core::repo`)

- **`Repo<S>`:** `new`, `shared(executor)` and `shared_async(executor)`.
- **`Acquire` / `AcquireAsync`:** connection sources used through `&self`.
  Pools will implement these.
- **`Shared<E>` / `SharedAsync<E>`:** one executor behind a mutex
  (std / `futures-util`); calls are serialized.
- **`impl Executor for &mut E`** (and the async equivalent), so
  `Repo::new(&mut tx)` works.

## Departures from the spec, agreed or noted

- **The trait is emitted almost as written.** The spec's `-> Option<User>`
  without `self` can't execute, so authors write honest signatures:
  - a receiver;
  - `async` where wanted;
  - `Result<R, E>`.
- **No `sync`/`async` cargo features.** Whether a repository is sync or async
  is now decided per trait by its methods.
- **`dyn_compatible` became the default**, and `static_dispatch` is the
  opt-out.
- **DSL capability bounds (milestone 6) will go on methods, not the trait.**

## Tests

- **Parser unit tests:** precedence, operators, errors.
- **SQL splitting:** quotes, comments, rejected placeholders.
- **`#[query]`:** 9 queries, including the spec's three examples.
  - Their rendered SQL is snapshotted, and all of them run on the memory
    backend.
  - 12 compile-error cases cover:
    - an unknown column, a wrong parameter type, and an unknown or unused
      parameter;
    - a JSON path on a scalar column, and a JSON column compared whole;
    - `bool` without `entity`, and chained comparisons;
    - a DSL call, `sql` combined with `filter`, `$1`, and a value used as a
      condition.
- **`#[repository]` on memory:**
  - dyn behind `Arc`, with a sync method blocking in place;
  - error kinds preserved through `DynError`;
  - sync `&self` over `Shared` with a user-defined error type;
  - `&mut self` inside a transaction;
  - `static_dispatch` (the future is `Send`);
  - a hand-written mock behind the same `Arc<dyn Trait>`.
- **`#[repository]` on Postgres:** `Arc<dyn Notes>` over `SharedAsync`,
  called from a spawned task, with a sync method blocking in place on a
  multi-threaded runtime.
- **9 repository compile-error cases:** mixed receivers, no receiver, a return
  type that isn't `Result`, a missing `#[query]`, a query method with a body,
  an unknown option, `static_dispatch` used as `dyn`, an error type without
  `From`, and a bad column.
  - On a macro error the trait is still emitted, so only the macro's own
    error appears.

## CRUD methods (agreed 2026-10-03)

CRUD methods are declared with flags on `#[query]`. Unlike fixed supertraits,
this keeps the receiver and `async` up to the author:

```rust
#[query(get)]                         async fn get(&self, id: i64) -> Result<Option<Note>, DynError>;
#[query(insert, returning)]           async fn create(&self, note: &NewNote) -> Result<Note, DynError>;
#[query(insert, returning)]           async fn create_many(&self, notes: &[NewNote]) -> Result<Vec<Note>, DynError>;
#[query(insert)]                      fn import(&self, note: &Note) -> Result<u64, DynError>;
#[query(update)]                      async fn patch(&self, patch: &NotePatch) -> Result<bool, DynError>;
#[query(delete)]                      async fn remove(&self, note: &Note) -> Result<bool, DynError>;
#[query(delete, entity = Note)]       async fn remove_id(&self, id: i64) -> Result<u64, DynError>;
#[query(delete, entity = Note, filter = body is_null)] fn purge(&self) -> Result<u64, DynError>;
```

- **One parameter.** It's taken by reference or by value; `&[V]` and
  `Vec<V>` insert several rows.
- **The entity comes from the signature:**
  - `get` and `insert, returning`: from the return type;
  - `insert`, `update` and value `delete`: inferred from the parameter's
    `Insertable` / `Updatable` / `Deletable` impl;
  - delete by id, or with a filter: from `entity = T`, because an id type
    doesn't name its entity.
- **Capabilities are checked at compile time.** For example, a parameter type
  that isn't `Insertable<_>` is reported as such.
- **Results:**
  - `get` returns `Option<T>`;
  - `insert, returning` returns `T` or `Vec<T>`;
  - `insert`, `update` and `delete` return `u64` or `bool`.

## Sync and async, in every combination

| repository methods | executor | how |
|---|---|---|
| async (+ sync methods) | async (tokio-postgres) | `Repo::shared_async(exec)`; sync methods block in place (`tokio` feature on a multi-thread runtime) |
| async (+ sync methods) | **sync** (memory, postgres, SQLite later) | `Repo::shared(exec)`: `Shared<E>` is also an `AcquireAsync`; futures complete immediately, no runtime needed |
| async, `&mut self` | sync executor or tx | `Repo::new(Blocking(&mut conn))` |
| sync only | sync | `Repo::shared(exec)` / `Repo::new(&mut conn)` |
| sync only | async | not supported: a sync trait over an async driver would need a runtime to block on; make the methods `async` |

`Blocking<E>` moved from the test suite into `rupa_core::exec`, and it adapts
transactions too.
