# Preparing for `rupa-upwell` (milestone 9)

Status: **items 1–3 done; item 4 partly** (2026-10-03). See
[`connections.md`](connections.md) for what was built and the decisions
taken. This note lists what rupa needs before the Upwell integration can be
built, so the work can start once Upwell is ready. Written 2026-10-03, after
milestone 8 and the CI stabilisation (`057d7af`).

| item | status |
|---|---|
| 1. Type-erased transactions | done: `BoxTx`/`BoxAsyncTx`, transactional `Box*Executor`, `try_begin_with_security`; owned transactions are option (a) |
| 2. Pools behind `Acquire`/`AcquireAsync` | done: `Pool`/`AsyncPool` in core; deadpool (tokio-postgres) and r2d2 (sync drivers) behind each driver's `pool` feature |
| 3. Descriptors and registry | done: `DbConfig` (typed `DriverConfig` enum), `Driver`, `Registry`, `Database`; option (a); explicit registration from facade features instead of `inventory` |
| 4. Smaller items | prepared-statement caching done (render caching open); sync MySQL driver done (sqlx still planned); `rupa-upwell` placeholder and release-plz workflow unchanged |

## Why wait

- Upwell's 1.0 is still some way off.
- Upwell's conditional components aren't ready, and the spec's milestone 10
  (and the `conditional` feature in milestone 9) build on them.
- Writing the adapter against a moving API would mean rewriting it.

Everything below is Upwell-independent, though. It keeps `rupa-core` and the
drivers DI-neutral (spec rule 6), and it's useful on its own.

## What Upwell expects (from its docs, 2026-10-03)

Source: `https://upwell-docs-production.up.railway.app/llms.txt`, and in
particular the *components* and *advanced dependency injection* pages.

- **Shared dependencies.** Components arrive as `Arc<T>`, and trait objects as
  `Arc<dyn Trait>` (`#[injectable]` traits,
  `#[component(provide = dyn Trait, qualifier = "..")]`).
- **Scopes:** singleton (the default), `upwell::daemon::Connection`,
  `upwell::daemon::Request`, and `transient`. A shorter-lived component may
  depend on a longer-lived one, never the reverse.
- **Construction:** `#[init]` methods inside `#[methods]` can be sync or
  async, and can return `Self` or `Result<Self, E>`.
- **Selection:** `primary`, `#[qualifier = ".."]`, and `Vec<Arc<dyn Trait>>`
  for fan-out.
- **Late binding:** `Dep<T>`, `Cfg<T>` (reloadable configuration), `Lazy<H>`,
  `Fresh<H>` and `Dynamic<H>`.

What this means for rupa:

- Repositories are already right for Upwell. `#[repository]` is
  dyn-compatible by default, with `&self` methods over a connection source.
- Executors and transactions, by contrast, are currently used through
  `&mut self` and aren't dyn-compatible when they're transactional.

## Prep work, in order

### 1. Type-erased transactions

**Needed for** request-scoped transaction components (`Arc<dyn ..>`), and
for executors whose dialect is chosen at run time (`BoxAsyncExecutor`) to be
transactional at all.

**Today:**

- `BoxExecutor` and `BoxAsyncExecutor` erase `Executor` / `AsyncExecutor`
  only.
- `Transactional` / `AsyncTransactional` have a lifetime-generic
  `type Tx<'t>` and generic methods, so they aren't dyn-compatible.

**Needed:**

- **A dyn-compatible transaction trait.** Something like `DynAsyncTransaction`
  with boxed-future `execute`, `begin_nested`, `commit` and `rollback`. Its
  blanket implementation would cover every `AsyncTransaction + AsyncTransactional`.
- **A boxed transaction type**, e.g. `BoxAsyncTx<'t>`, implementing
  `AsyncExecutor + AsyncTransaction + AsyncTransactional`. Repositories would
  then run in it unchanged, and `BoxAsyncExecutor` could become
  `AsyncTransactional` with `Tx<'t> = BoxAsyncTx<'t>`.
- **A runtime-checked `try_begin_with_security`** for erased executors. It
  checks `dialect.supports(NativeRls)` and returns an error, instead of the
  compile-time bound.
- **Sync equivalents**, for symmetry.

**Open question: ownership for request scope.** A transaction borrows its
connection (`Tx<'t>`). A request-scoped component must be `'static`
(`Arc<..>`), so it would have to own its connection. The pool from item 2
provides that: the component holds a pooled connection and its open
transaction together. Options:

- **(a)** An owning transaction type that holds the pooled connection, e.g.
  `PooledTx` (connection plus transaction state).
- **(b)** A self-borrowing wrapper. This is error-prone; avoid it.

Recommendation: **(a)**. Transaction state (depth, done, pending rollback)
already lives in the driver types, so an owning variant mostly moves the
connection inside.

### 2. A connection pool behind `Acquire` / `AcquireAsync`

**Needed for** a singleton executor component. Today the only connection
source is `Shared` / `SharedAsync`: one connection behind a mutex, so every
call is serialized.

**Needed:**

- `AcquireAsync` / `Acquire` implementations backed by a real pool. Each
  `acquire` hands out a pooled connection that implements the executor trait.
- **Dirty-connection handling on return.** A connection whose transaction was
  dropped (`is_dirty()`) must be cleaned (`clean()`, i.e. `ROLLBACK`) or
  discarded before reuse. The milestone 4 note promised this hook. The pool's
  recycle check is where it goes.
- **An owning transaction from the pool**, for item 1:
  `pool.begin_with(options)` / `begin_with_security(..)` returning a
  `'static` transaction.

**Decide:** which pool, per driver.

- Use an existing pool:
  - `deadpool-postgres` for tokio-postgres;
  - `r2d2` / `r2d2_sqlite` for the sync drivers;
  - for sqlx later, its own pool.
- Or write a small pool in rupa.

Recommendation: wrap existing pools in each driver crate, behind a cargo
feature, so the core stays dependency-light.

### 3. Driver descriptors and a registry, DI-neutral

**Needed for** choosing a driver from configuration
(`Cfg<DbConfig>` → executor), which the spec asks of `rupa-upwell`. The
descriptor and registry don't involve Upwell, so they can live outside
`rupa-upwell` (in `rupa-core` or a small `rupa-registry` crate), and
`rupa-upwell` only consumes them.

**Needed:**

- **A `DbConfig` type:** driver/dialect id, connection URL or parameters,
  pool settings, and an optional qualifier name.
- **A descriptor trait per driver:** its dialect id, plus a build function.
  Each driver crate registers its descriptor via `inventory` (or `linkme`)
  when its own feature is enabled.
- **A resolver** that matches `DbConfig` to a registered descriptor and builds
  the executor or pool.

**Decide (raised in the spec, still open):**

- **(a) One registry where every build is async and fallible:**
  `fn build(&DbConfig) -> BoxFuture<Result<Arc<dyn DynAsyncSource>, Error>>`.
  It's simpler, and sync drivers fit through `Shared` / `Blocking`, as
  milestone 5 showed.
- **(b) Mirrored sync and async registries:** this matches the
  `Executor` / `AsyncExecutor` split, at the cost of more surface.

Recommendation: **(a)**. Sync drivers already serve async repositories without
a runtime, and Upwell's `#[init]` supports async, fallible construction.

Also: qualifiers (`primary` / `replica`, or one per dialect) map to several
`DbConfig`s. Document this as a recipe next to the registry.

### 4. Smaller items found along the way

- **Statement caching** (agreed in milestone 2, deferred): render once per
  query site and dialect, plus prepared statements in drivers. It matters more
  once pools serve many requests.
- **The MySQL driver** (sqlx, milestone 11). Its behaviour is already pinned
  down by the test-only executor in `crates/rupa-conformance/tests/mysql.rs`
  (`CLIENT_FOUND_ROWS`, statement-by-statement transaction scripts).
- **`rupa-upwell` is publishable but empty.** The facade's optional dependency
  requires that. Decide whether to publish a placeholder 0.1.0 or hold
  releases until it has content.
- **No release-plz workflow yet.** `release-plz.toml` exists, but there's no
  GitHub workflow; it needs a crates.io token secret.

## What waits for Upwell

These are the spec's milestones 9 and 10 proper, once Upwell's APIs settle:

- **Executor component:** a `#[component(provide = dyn ..)]` constructor
  calling the resolver from item 3. Later it'll be replaced by Upwell's
  conditional components (the `conditional` feature), with the same resolver
  underneath.
- **Repository components:** a helper macro generating a newtype around
  `Repo<Source>` that forwards the dyn-compatible trait and carries
  `#[component(provide = dyn Trait, qualifier = ..)]`.
- **Transaction component:** request- or connection-scoped (never singleton),
  built on items 1 and 2.
- **RLS context:** one request-scoped component holding the `SecureTx`, which
  already carries its `SecurityContext`. Never two separately injectable
  components.
- **Example app** as an integration test, in a separate CI job behind
  `--features upwell`. The job exists and is currently a no-op.
