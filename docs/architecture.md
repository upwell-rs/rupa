# Architecture: how a query flows

This is the map of the workspace: which crate owns what, and what happens,
file by file, between calling a repository method and getting rows back.

## Layers

```mermaid
flowchart TB
    app["your crate<br/>#[derive(Entity)] · #[repository] · #[query]"]
    macros["rupa-macros<br/>proc-macros: generate builder calls"]
    facade["rupa (facade)<br/>re-exports, cargo features, registry()"]
    core["rupa-core (sans-IO)<br/>Value · IR · Query&lt;R&gt; · builder · traits"]
    sql["rupa-sql<br/>IR → SQL + params · render cache"]
    dsl["rupa-dsl-std<br/>built-in DSL functions"]
    drivers["driver crates<br/>tokio-postgres · postgres · rusqlite · mysql · sqlx · memory"]
    db[("database")]

    app --> facade
    app -. expands via .-> macros
    facade --> core & sql & dsl & drivers
    drivers --> core
    drivers -- "SQL drivers" --> sql
    drivers --> db
```

- **`rupa-core`** never does IO. It defines the data (`Value`, the IR in
  `ir.rs`, `Query<R>`), the typed builder that produces it, and the traits a
  driver implements (`Executor`, `TxConnection`, `Pool` and their async
  twins).
- **`rupa-sql`** turns a `Statement` into SQL text plus bound parameters for
  one dialect. Only SQL drivers use it; the memory backend evaluates the IR
  directly.
- **Driver crates** run statements and decode rows. Each one is small:
  binding (`Value` → driver type), reading (driver row → `Value`), its error
  type (`rupa_sql::driver_error!`), and a pool.
- **`rupa`** is the facade. It owns the feature graph and, with `dynamic`,
  the registry of enabled drivers.

## One call, end to end

Take this repository, run on a SQLite pool:

```rust
#[repository]
pub trait Users: Send + Sync {
    #[query(filter = email == $email)]
    fn by_email(&self, email: &str) -> Result<Option<User>, rupa::DynError>;
}

let pool = SqlitePool::connect(&config)?;
let users = UsersView::new(&pool);
users.by_email("a@b.c")?;
```

### 1. Compile time: the macros write builder code

`#[derive(Entity)]` (`rupa-macros/src/entity.rs`) implements `Entity`
(table, columns, ids) and `FromRow` (decode by position) for `User`, plus a
hidden probe per field that `col!(User::email)` uses to find the column's
type and kind (`rupa-core/src/column/__private.rs`).

`#[repository]` (`rupa-macros/src/repository.rs`) emits the trait, a unique
`UsersKey`, a borrowed `UsersView<'_, S>`, and an owned `UsersRepo<S>`.
The key prevents unrelated repository traits from accumulating on one
concrete type. For each method:

- `query::query_body` (`rupa-macros/src/query/mod.rs`) translates
  `#[query(filter = ..)]` into builder calls: `select::<User>().filter(..)
  .optional()`. The filter grammar is parsed by `dsl.rs` and turned into
  `Expr` code by `query/ctx.rs`. The result type (`Option<User>`) picks the
  terminal (`.optional()`) in `query/signature.rs`.
- An `&self` method acquires from a pool or transaction session. Acquisition
  and execution errors pass through `DynError`; the declared error type only
  needs `From<DynError>`. Sync or async
  is decided by whether any method is `async`.

So the method body is roughly:

```rust
let __query = select::<User>().filter(col!(User::email).eq(email)).optional();
let mut __conn = Acquire::acquire(self.source()).map_err(DynError::new)?;
Ok(Executor::run(&mut __conn, __query).map_err(DynError::new)?)
```

### 2. Building: a `Query<R>` value, no IO

`select::<User>()` (`rupa-core/src/builder.rs`) starts an
`ir::Select` with the entity's table and projection. `col!(User::email)
.eq(email)` builds an `Expr<bool>` (`expr.rs`) whose node is
`ExprNode::Binary(Eq, Column(email), Param(Value::Text(..)))`. Values only
ever enter the IR as `ExprNode::Param`, so they are always bound, never
spliced into SQL.

`.optional()` finishes it as `Query<Option<User>>` (`query.rs`): the
statement plus `Expect::AtMostOne`, so the driver can stop after two rows.

### 3. Checking out a connection

`SqlitePool` implements `Pool` through `rupa_core::r2d2_pool!`
(`rupa-core/src/pool/r2d2.rs`), and every `Pool` is an `Acquire` source
(`pool/mod.rs`). `acquire()` returns a `Pooled<r2d2::PooledConnection<..>>`,
which forwards `Executor` to the `SqliteExecutor` inside and returns the
connection to the pool when dropped.

### 4. Executing: render (cached), prepare (cached), run

`Executor::run` (`rupa-core/src/exec/mod.rs`) calls the driver's
`execute(statement, expect)` and decodes the `Outcome`.
`SqliteExecutor::run` (`rupa-driver-rusqlite/src/lib.rs`):

1. `clean()`: sends a rollback still owed by a transaction dropped earlier
   (see "Transactions").
2. `rupa_sql::render_cached(statement, dialect)`
   (`rupa-sql/src/cache/mod.rs`):
   - `cache/shape.rs` walks the statement once and produces its **shape**:
     a token stream of node kinds, identifiers (by `&'static str` address),
     and a bare "parameter" token per value. Values are never part of it.
   - The shape's hash picks one of 16 shards (`Mutex<hashlink::LruCache>`).
     On a hit, the cached SQL text (`Arc<str>`) is reused, and this call's
     values are bound through the entry's binding plan.
   - On a miss, `writer/` renders the SQL with the dialect's `Syntax`
     (`postgres.rs`, `mysql.rs`, `sqlite.rs`), recording which IR value
     feeds each placeholder. That record becomes the binding plan.
   - DSL calls are lowered (`cache/lower.rs`) before shaping, so the shape
     is that of the SQL actually rendered.
3. `conn.prepare_cached(&sql)`: rusqlite's own per-connection statement
   LRU. tokio-postgres keeps a `StatementCache` per connection
   (`rupa-driver-tokio-postgres/src/shared.rs`), sqlx and mysql their own.
4. Binds each `Value` through `value::Param` (text and blobs borrowed, not
   copied), runs, and buffers up to `expect.fetch_limit()` rows into a
   `RowCursor`.

### 5. Decoding: rows to `R`

`Outcome::decode` (`exec/mod.rs`) hands the cursor to
`<Option<User> as QueryResult>::from_output` (`query.rs`), which takes at
most one row, errors on a second (`ResultError::TooManyRows`), and calls
`User::from_row`. That reads each column by position as its `SqlType`
(`Row::get`), and the driver converts its own value into a `Value`
(`rupa-driver-rusqlite/src/value.rs`). Errors come back as the driver's
`SqliteError`, wrapped in `DynError`, then converted to the method's `E` by
`From<DynError>`.

## Static and dynamic

```mermaid
flowchart LR
    subgraph static ["default: statically typed"]
        R1["Repo&lt;SqlitePool&gt;"] --> P1["SqlitePool (Pool)"] --> E1["SqliteExecutor"]
    end
    subgraph dynamic ["feature dynamic"]
        C["DbConfig"] --> REG["rupa::registry()"] --> D["Driver::connect"] --> DB["Database (Arc&lt;dyn DynPool&gt;)"]
        R2["Repo&lt;Database&gt;"] --> DB --> BE["BoxAsyncExecutor"] --> E2["any driver's executor"]
    end
```

- **Static (default).** The repository is generic over the pool, so every
  call is monomorphized: no boxing, no dynamic dispatch per statement.
- **Dynamic (`dynamic`).** `rupa::connect(&config)` resolves a driver by
  name or URL scheme (`rupa-core/src/registry/mod.rs`) and returns a
  `Database`: an `Arc<dyn DynPool>` whose connections are
  `BoxAsyncExecutor`s (`erased/`). Errors are `DynError`. Sync drivers are
  served through `Blocking`. This path costs a box and a virtual call per
  statement (`docs/benchmarks.md`: about 0.3 µs on SQLite).

## Sync and async

Each runtime contract has a sync half and an async half behind the `async`
feature: `exec/`, `tx/`, `pool/`, `repo/` and `security/` each keep the
async half in an `asynchronous.rs` submodule. No runtime is involved until a
driver needs one: tokio-postgres and sqlx spawn on tokio, and the facade's
`tokio` feature only changes how a sync method of an async repository
blocks (`rupa/src/__macro_support.rs`).

`Blocking<E>` adapts a sync executor or pool to the async traits: futures
that are already complete. That is how async repositories run on SQLite
without a runtime.

## Transactions

`tx/mod.rs` holds the guard (`Tx<C>`) and the traits; drivers implement only
`TxConnection::control` (send `BEGIN`/`COMMIT`/`SAVEPOINT`, rendered by
`rupa_sql::render_tx`) and the dirty-connection bookkeeping:

- `begin()` on a transaction opens a savepoint (`depth + 1`).
- A sync `Tx` dropped while open rolls back in `Drop`.
- An `AsyncTx` can't await in `Drop`. It marks the connection dirty
  (`PendingRollback`), the driver sends the rollback before its next
  statement, and pools `clean()` connections when they come back.
- `begin_with_security` (`security/`) applies row-level-security settings
  inside the transaction. It only compiles for dialects with native RLS
  (`Supports<caps::NativeRls>`).

## Joins

`select::<A>().inner_join_on::<B>(|(a, b)| ..)` turns the builder into a
`JoinBuilder<(Src<A, 0>, Src<B, 1>), (A, B)>` (`join/build.rs`). Each source
is a handle that columns are taken through, so every column names its
source. Rows decode into tuples (`join/decode.rs`) or a `#[derive(Joined)]`
struct (`join/merged.rs`). Declared relations (`#[column(references = ..)]`)
let `join::<B>()` infer the condition (`join/relation.rs`).

## Where to look

| to change | look in |
|---|---|
| what a query can express | `rupa-core/src/ir.rs`, then `builder.rs` / `expr.rs` |
| the `#[query]` grammar | `rupa-macros/src/dsl.rs`, `query/attr.rs`, `query/ctx.rs` |
| the SQL a dialect gets | `rupa-sql/src/writer/` and `postgres.rs` / `mysql.rs` / `sqlite.rs` |
| caching | `rupa-sql/src/cache/`, `docs/design/caching.md` |
| a driver's value mapping | `<driver>/src/value.rs` (sqlx: `<db>/mod.rs`) |
| pools | `rupa-core/src/pool/` (traits and r2d2/deadpool glue), `<driver>/src/pool.rs` |
| configuration | `rupa-core/src/config/` |
| run-time driver selection | `rupa-core/src/registry/`, `rupa/src/registry.rs` |
| behaviour every backend must share | `rupa-conformance` |
