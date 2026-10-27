# rupa-core

## Role

The sans-IO core: values, the query IR, the typed builder, and the entity,
row, dialect, executor, transaction, pool and repository traits. Nothing here
opens a connection.

## Usage

Applications use it through the `rupa` facade. Driver crates implement its
traits: `Executor` / `AsyncExecutor` (run one statement), `TxConnection` /
`AsyncTxConnection` (transaction control), and `Pool` / `AsyncPool`.

Queries run with `query.execute(&mut engine)`. That returns `Result<R, E>` on
a sync engine and a future of it on an async one. Every executor is an
`Engine` naming its `Completion`: `Executor` requires `Immediate`, and
`AsyncExecutor` requires `Awaited`. A driver adds one line next to its executor
impl, for example `impl Engine for MyExecutor { type Completion = Immediate; }`.
The async future is boxed, because stable Rust cannot yet name an `async fn`'s
future in an associated type. Generated async repository methods stay unboxed
through the hidden `complete_unboxed`.

## Internal role

- `builder`, `expr`, `column`, `join` build `Query<R>` values; `ir` is what
  they produce and what renderers and the memory backend consume.
- `schema` is the dialect-free DDL model (tables, columns, keys, indexes)
  that migration plans hold, with builders from literals or entity metadata.
- `exec`, `tx`, `pool`, `repo`, `security` are the runtime contracts. Each has
  a sync half and, behind `async`, an `asynchronous` submodule.
- `erased` and `registry` (behind `dynamic`) erase executors and pools for
  run-time driver selection.
- `config` is the serde-friendly `DbConfig` every driver's `connect` reads.
- `pool::r2d2` / `pool::deadpool` and the `r2d2_pool!` / `blocking_driver!`
  macros are glue the driver crates share.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID `Value`s and column types; `chrono_text` |
| `async` | async executors, transactions, pools, repositories, `BoxFuture` |
| `dynamic` | `erased` (boxed executors); with `async`, `registry` (`Registry`, `Driver`, `Database`) |
| `r2d2` | `pool::r2d2`: builder, status and the `r2d2_pool!` macro for sync drivers |
| `deadpool` | `pool::deadpool`: builder, checkout, status and `Expiry` for async drivers (implies `async`) |
