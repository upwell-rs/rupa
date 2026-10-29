# rupa-core

## Role

The sans-IO core: values, the query IR, the typed builder, and the entity,
row, dialect, executor, transaction, pool and repository traits. Nothing here
opens a connection.

## Usage

Applications use it through the `rupa` facade. Driver crates implement its
traits through `&self`: `Executor` / `AsyncExecutor` (run one statement),
`TxConnection` / `AsyncTxConnection` (transaction control), `Transactional` /
`AsyncTransactional` (four lines: `Tx::begin(self, options)`), and `Pool` /
`AsyncPool` with `pool_executor!`. A driver whose native API needs `&mut`
serializes inside its executor; one whose client is shareable needs nothing.

Every executor is an `Engine` naming its `Completion`: `Executor` requires
`Immediate`, and `AsyncExecutor` requires `Awaited`. From those, every
transactional engine is a `Connection`, the one trait repositories and
generic code use: `run` a statement and `begin_tx` a transaction, both
returning futures. An `Immediate` connection's futures are already complete,
so sync code takes their value with `now`, without a runtime or boxing.
Executors are forwarded through `&`, `&mut`, `Box`, `Rc` and `Arc`.

Queries run with `query.execute(&connection)`. That returns `Result<R, E>` on
a sync connection and a boxed future of it on an async one, because stable
Rust cannot yet name an `async fn`'s future in an associated type. Repository
methods stay unboxed: they call `Connection::run` directly.

## Internal role

- `builder`, `expr`, `column`, `join` build `Query<R>` values; `ir` is what
  they produce and what renderers and the memory backend consume.
- `schema` is the dialect-free DDL model (tables, columns, keys, indexes)
  that migration plans hold, with builders from literals or entity metadata.
- `exec`, `tx`, `pool`, `security` are the runtime contracts. Each has a sync
  half and, behind `async`, an `asynchronous` submodule. `connection` unifies
  them as `Connection`, and `repo` holds `DynRepository`, what
  `#[repository]` builds on.
- `erased` and `registry` (behind `dynamic`) erase executors and pools for
  run-time driver selection.
- `config` is the serde-friendly `DbConfig` every driver's `connect` reads.
- `pool::r2d2` / `pool::deadpool` and the `r2d2_pool!` / `pool_executor!` /
  `blocking_driver!` macros are glue the driver crates share.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID `Value`s and column types; `chrono_text` |
| `async` | async executors, transactions, pools, repositories, `BoxFuture` |
| `dynamic` | `erased` (boxed executors); with `async`, `registry` (`Registry`, `Driver`, `Database`) |
| `r2d2` | `pool::r2d2`: builder, status and the `r2d2_pool!` macro for sync drivers |
| `deadpool` | `pool::deadpool`: builder, checkout, status and `Expiry` for async drivers (implies `async`) |
