# Connections: pools, configuration and erased transactions

Status: **implemented** (2026-10-03). This is items 1–3 of
[`upwell-prep.md`](upwell-prep.md), plus the cleanups they made possible.
Everything here is DI-neutral; `rupa-upwell` will only consume it.

## Transactions live in core once

Before, every driver carried its own copy of the transaction guard
(`PgTx`, `SqliteTx`, `MemoryTx`, the MySQL test executor's `MyTx`): depth,
`done` flag, savepoint naming, drop behaviour. Now drivers implement one
small trait, and `rupa_core::tx` does the rest:

```rust
pub trait TxConnection: Executor<Error: From<TxError>> + Send {
    fn control(&mut self, statement: TxStatement) -> Result<(), Self::Error>; // BEGIN, SAVEPOINT, ..
    fn defer_rollback(&mut self, depth: u32);   // a rollback owed before the next statement
    fn is_dirty(&self) -> bool;
    fn clean(&mut self) -> Result<(), Self::Error>;
}
// + AsyncTxConnection, the same with futures.
```

- `Tx<C>` / `AsyncTx<C>` are the guards, generic over how they hold the
  connection: `&mut Conn` (what `begin()` returns) or an owning handle
  (`Box<Pooled<..>>`, what a pool's `begin()` returns). Owning is what lets a
  transaction be `'static`, for a request scope. This is option (a) of the
  prep note's ownership question; no self-borrowing anywhere.
- Every `TxConnection` is `Transactional` through a blanket impl, so the
  `Tx<'t>` type of every driver is `Tx<&'t mut ItsExecutor>`. The old names
  stay as aliases (`PgTx<'t>`, `SqliteTx<'t>`, ...).
- `PendingRollback` is the shared bookkeeping for deferred rollbacks.
- `Blocking<E>` (a sync connection used from async code) is now an
  `AsyncTxConnection` itself; dropping its async transaction rolls back on
  the spot, as a sync one would.

## Type erasure (prep item 1)

`rupa_core::erased`:

| static | erased |
|---|---|
| `E: Executor + Transactional` | `BoxExecutor` |
| `E::Tx<'t>` | `BoxTx<'t>` |
| `E: AsyncExecutor + AsyncTransactional` | `BoxAsyncExecutor` |
| `E::Tx<'t>` | `BoxAsyncTx<'t>` |

The boxed types implement the static traits again (`Dialect = DynDialect`,
`Error = DynError`), so `begin` on a `BoxAsyncExecutor` gives a `BoxAsyncTx`,
nested `begin` gives savepoints, and repositories run in them unchanged.

- **Erased executors must be transactional.** Every real connection is; it
  keeps one boxed type instead of two.
- **`try_begin_with_security`** is the runtime-checked twin of
  `begin_with_security`, available on every transactional executor. It fails
  with `TxError::Unsupported { dialect, capability: NativeRls }` before
  touching the connection.
- **`DynError`** gained `other(..)`, `downcast_ref` and `get_ref`, so pool
  timeouts and the like can be carried and inspected.

## Pools (prep item 2)

Core defines what a pool is; driver crates wrap established pools behind a
`pool` feature (the prep note's recommendation):

| driver | pool | why |
|---|---|---|
| `rupa-driver-tokio-postgres` | deadpool (`managed`, tokio) | async, runtime-aware timeouts |
| `rupa-driver-postgres` | r2d2 | sync |
| `rupa-driver-rusqlite` | r2d2 | sync |
| `rupa-driver-mysql` | r2d2 | sync |

```rust
pub trait AsyncPool: Send + Sync + 'static {   // + `Pool`, the sync twin
    type Conn: AsyncTxConnection + 'static;     // usually Pooled<the pool's guard>
    fn dialect(&self) -> &<Self::Conn as AsyncExecutor>::Dialect;
    fn get(&self) -> impl Future<Output = Result<Self::Conn, ..>> + Send;
    fn status(&self) -> PoolStatus;
    // provided: begin / begin_with -> OwnedAsyncTx<Conn>, begin_with_security (static gate)
}
```

- **Free with it:** `Acquire` / `AcquireAsync` (blanket impls, so
  `Repo<ThePool>` works), owned transactions, statically gated
  `begin_with_security`, and erasure into `Database`.
- **`Pooled<G>`** wraps a pool's guard and forwards every executor and
  transaction trait. The orphan rule requires the wrapper, and it means
  drivers write no forwarding code.
- **`Blocking<P>`** turns a sync pool into an async one, as `Blocking<E>`
  does for an executor.
- **`SingleConnection<E>`** is one connection as a pool of size one, for the
  memory backend and tests.
- **Dirty connections:**
  - deadpool's `recycle` and r2d2's `has_broken` clean a connection that owes
    a rollback, and discard it if that fails;
  - drivers also send the rollback before the next statement, so even a pool
    without the hook stays correct;
  - Postgres's `recycle` also enforces `idle_timeout` / `max_lifetime`
    (r2d2 does that itself).
- **Fail fast.** r2d2's `build()` retries a failing connection until the
  acquire timeout and then reports only "timed out". The r2d2 pools open one
  probe connection first, so a bad path or password fails immediately, with
  the driver's error.

## Configuration and the registry (prep item 3)

### `DbConfig`: shared fields plus a typed driver enum

```rust
pub struct DbConfig {
    pub url: String,
    pub pool: PoolConfig,             // max_size, min_idle, acquire/idle timeouts, max_lifetime
    pub qualifier: Option<String>,    // `primary`, `replica`: carried for DI
    pub driver: DriverConfig,
}

#[non_exhaustive]
pub enum DriverConfig {
    Auto,                             // chosen by URL scheme, default options
    Postgres(PostgresOptions),        // async
    PostgresSync(PostgresOptions),
    Sqlite(SqliteOptions),
    MySql(MySqlOptions),
    Memory,
    Other { name: String, options: BTreeMap<String, String> },  // third-party drivers
}
```

Serialized, the driver is a flattened `driver` tag with its options next to
the shared fields:

```toml
url = "sqlite://data/app.db"
driver = "sqlite"        # optional
busy_timeout = "2s"
journal_mode = "wal"
[pool]
max_size = 8
```

- **Typed.** Known drivers' options are checked against their struct
  (`deny_unknown_fields`). A typo is a deserialization error, not a silently
  ignored key.
- **Inferred tag.** Without `driver`, options are typed by the URL scheme's
  default driver. With neither tag nor options, the config is `Auto`.
- **Fallback.** An unknown tag becomes `Other`, with its options as strings.
  `DriverOptions` reads them and reports leftover keys.
- **Portable.** The variants exist whether or not their driver is compiled
  in, so one configuration file serves every build. The registry reports a
  driver that is missing from the build.
- **Durations** are milliseconds or `"500ms"` / `"30s"` / `"5m"` / `"1h"`,
  and `"off"` / `null` disables a timeout. Passwords are redacted in `Debug`.

### Registry

- **`Driver`** is one per driver crate: its name (the `driver` tag), dialect,
  URL schemes, and `connect(&DbConfig) -> BoxFuture<Result<Database,
  ConnectError>>`.
- **`Registry`** is a list of drivers:
  - resolution is by name when the config names a driver, otherwise the first
    driver registered for the URL scheme;
  - errors list what is registered, as `name (schemes)`.
- **Decided: one async, fallible registry** (option (a)). Sync drivers serve
  through `Blocking`, as repositories already do.
- **Decided: no `inventory` / `linkme`.** Registration is explicit. The facade
  builds its registry from cargo features, once:

```rust
pub fn registry() -> &'static Registry   // OnceLock; drivers are unit structs
pub async fn connect(config: &DbConfig) -> Result<Database, ConnectError>
```

| facade feature | driver name | crate |
|---|---|---|
| `postgres` | `postgres` | tokio-postgres + deadpool |
| `postgres-sync` | `postgres-sync` | postgres + r2d2 |
| `sqlite` | `sqlite` | rusqlite + r2d2 |
| `mysql` | `mysql` | mysql + r2d2 |
| `memory` | `memory` | the in-memory backend |
| `full` | all of the above | |

**Update (2026-10-18):** the registry, `Database` and each driver's registry
entry are behind the facade's `dynamic` feature (`full` includes it). Most
applications know their database and use its pool directly, statically
typed; the registry is for configurations that pick one at run time.

Feature, `driver` tag and registry name are the same word. For `postgres:`
URLs the async driver is registered first, so it wins unless the config says
`driver = "postgres-sync"`.

### `Database`

The registry's product: an `Arc` around an erased pool.

- `Clone` is cheap and shares the pool.
- It is `AcquireAsync`, so `Arc::new(Repo::new(db))` serves
  `Arc<dyn Repository>`.
- `begin()` returns a `BoxAsyncTx<'static>` that owns its connection.
- `try_begin_with_security` is checked at run time.
- `status()` reports occupancy.

This is the `Arc<dyn ..>` the Upwell executor component will provide.

## Also done

- **MySQL driver** (`rupa-driver-mysql`), promoted from the test-only
  executor. It is sync, on the `mysql` crate, and sets `CLIENT_FOUND_ROWS`
  itself.
- **sqlx drivers** (`rupa-driver-sqlx`): async Postgres, MySQL and SQLite;
  see [`sqlx.md`](sqlx.md).
- **Prepared-statement caching** (half of prep item 4):
  - both Postgres drivers keep a per-connection LRU of prepared statements
    (`StatementCache`, 256 entries). A cached query is one round trip instead
    of two, and a statement whose execution fails is evicted in case it is
    stale;
  - SQLite uses rusqlite's `prepare_cached` (128 entries);
  - the `mysql` crate caches by itself.
- **Conformance through the pool.** `rupa_conformance::run_database` runs the
  executor and transaction suites on a `BoxAsyncExecutor` from a `Database`,
  then pool cases:
  - owned transactions;
  - a dropped owned transaction rolled back before the connection is reused;
  - an `Arc<dyn Repository>` over the pool;
  - runtime RLS gating;
  - connections returning to the pool.

  Every driver runs it.
- **Benchmarks** (`crates/rupa-bench`, see [`../benchmarks.md`](../benchmarks.md)).
- **Docs.** `#![warn(missing_docs)]` on the core, renderer, DSL, driver and
  facade crates (not yet on `rupa-macros` or the empty `rupa-upwell`).

## Still open

- ~~**Render caching.**~~ Done since: see [`caching.md`](caching.md).
- **TLS** for the registry's Postgres drivers. Today it is programmatic only:
  `PgManager::with_tls` and `Database::new(pool)`.
- **The memory backend clones whole tables** on `begin` (snapshots) and on
  every insert. Fine for tests, visible in the benchmarks (~1 ms per
  transaction at 1000 rows).
- **The `rupa-upwell` placeholder and the release-plz workflow** (prep item 4)
  are unchanged.
