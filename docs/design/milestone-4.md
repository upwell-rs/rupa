# Milestone 4: transactions

Status: **implemented.**

## API (`rupa_core::tx`)

```rust
pub trait Transactional: Executor {
    type Tx<'t>: Transaction + Transactional + Send + Executor<Dialect = .., Error = ..> where Self: 't;
    fn begin_with(&mut self, options: TxOptions) -> Result<Self::Tx<'_>, Self::Error>;
    fn begin(&mut self) -> ..;                                    // default options
    fn transaction<'a, T, E, F>(&'a mut self, f: F) -> Result<T, E>
        where F: FnOnce(&mut Self::Tx<'a>) -> Result<T, E>, E: From<Self::Error>;
}
pub trait Transaction: Executor { fn commit(self) -> ..; fn rollback(self) -> ..; }
// Async mirror: AsyncTransactional / AsyncTransaction; `transaction` takes an AsyncFnOnce.
```

- **Guard:** `begin()` returns a `Tx` that is itself an executor, so
  repositories generic over `Executor`/`AsyncExecutor` work unchanged inside
  or outside a transaction.
- **Nesting:** `begin()` on a `Tx` opens a savepoint (`rupa_sp_<depth>`).
  `commit` releases it; `rollback` rolls back to it and releases it.
- **Closure:** commits on `Ok`. On `Err` it rolls back and returns the
  original error. On a panic (sync), unwinding drops the `Tx`, which rolls
  back.
- **Options:** `TxOptions { isolation, read_only }`. Setting them on a nested
  transaction is an error (`TxError::OptionsOnNested`), not ignored.
- **Control statements are rendered per dialect** by `rupa_sql::render_tx`
  (`BEGIN ISOLATION LEVEL .. READ ONLY`, `SAVEPOINT`, `RELEASE`,
  `ROLLBACK TO`). They are not delegated to each driver crate's own
  transaction type, so the same state machine serves every SQL driver.

### Design notes
- **The closure's `Tx` borrows from the call (`Tx<'a>`).** A higher-ranked
  lifetime combined with the GAT's `where Self: 't` would require
  `Self: 'static`, which breaks nested `transaction` calls on a `Tx`. A test
  covers this.
- **Sync `Tx` types must be `Send`.** This lets a sync transaction be driven
  from async code (the conformance adapter, and later DI). Every planned sync
  driver connection is `Send`.
- **When the async `transaction` future is `Send`.** The trait can't name
  `AsyncFnOnce`'s future type on stable, so the trait itself promises no
  `Send`. With a concrete executor type, the compiler sees that the future is
  `Send` (auto traits leak through). Generic code that must spawn can use the
  guard API.

## Rollback when a `Tx` is dropped
- **Sync:** `Drop` sends the rollback immediately. If that fails, the
  executor is marked dirty.
- **Async:** `Drop` can't await. It marks the executor dirty with the depth
  to roll back to; if several are pending, the shallowest wins. Then:
  - every later statement, `begin`, `commit` or `rollback` on that connection
    first sends the owed `ROLLBACK` or `ROLLBACK TO SAVEPOINT`;
  - `PgExecutor::is_dirty()` and `clean()` let a pool check and clean a
    connection before reuse (M9).
- **Cancellation:** `commit` and `rollback` set the "done" flag only after the
  statement succeeds. A cancelled or failed finish therefore still leaves the
  rollback owed.
- **Known edge case:** a savepoint `RELEASE` cancelled after it reached the
  server leaves an owed `ROLLBACK TO` for a savepoint that no longer exists.
  That surfaces as an error on the next statement rather than silently.

## Backends
- **Postgres (both drivers):** `PgTx<'t>` holds the executor and its depth. A
  tested case shows that a dropped async transaction leaves the connection
  dirty, and that after `clean()` the server has no open transaction.
- **Memory:** a stack of table snapshots. Commit drops a snapshot; rollback
  restores it. Read-only is enforced (`MemoryError::ReadOnly`), and every
  isolation level behaves as serializable (single-threaded). It differs from
  Postgres in one way: identity sequences roll back with the transaction,
  whereas Postgres doesn't reuse sequence values.

## Tests
- **Conformance:** `run_transactions` adds 12 cases:
  - commit, rollback, and drop;
  - a `Tx` used as a generic executor;
  - three nesting scenarios;
  - the closure API (`Ok` and `Err`);
  - read-only;
  - options on a nested transaction;
  - isolation levels;
  - transactions in sequence after a drop.

  All pass on memory (sync and async) and both Postgres drivers. The sync ones
  run through `Blocking`, which now also adapts the transaction traits.
- **Memory sync tests:** the closure API, nested closures, and rollback on
  panic.
- **Snapshot of the rendered control statements.**

## Not yet
- **Type-erased transactions:** `BoxExecutor` / `BoxAsyncExecutor` aren't
  transactional. `rupa-upwell`'s request-scoped transactions (M9) will need an
  object-safe transaction trait.
- **Pools:** not covered; the dirty flag is the hook they'll use.
