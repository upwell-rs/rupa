# rupa-core

## Role

The sans-IO core: values, the query IR, the typed builder, and the entity,
row, dialect, executor, transaction, pool and repository traits. Nothing here
opens a connection.

## Usage

Applications use it through the `rupa` facade. Driver crates implement its
traits: `Executor` / `AsyncExecutor` (run one statement), `TxConnection` /
`AsyncTxConnection` (transaction control), and `Pool` / `AsyncPool`.

## Internal role

- `builder`, `expr`, `column`, `join` build `Query<R>` values; `ir` is what
  they produce and what renderers and the memory backend consume.
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
