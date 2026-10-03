# Milestone 2: executors and drivers

Status: **implemented.** Rendered SQL is re-rendered on every call (agreed
2026-10-03); per-query-site caching is a planned follow-up.

## Executor traits (`rupa_core::exec`)

```rust
pub trait Executor {
    type Dialect: Dialect;
    type Error: ExecError;
    fn dialect(&self) -> &Self::Dialect;
    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, Self::Error>;
    fn run<R: QueryResult>(&mut self, query: Query<R>) -> Result<R, Self::Error> { /* execute + decode */ }
}

pub trait AsyncExecutor: Send {
    // same shape; `execute` returns `impl Future<Output = ..> + Send`, `run` is provided
}
```

- **Drivers implement only `execute`.** It returns an owned `Outcome`: either
  `Rows(Box<dyn RowCursor + Send>)` or `Affected(u64)`. Decoding into `R` is
  shared code.
- **`Expect` tells the driver how much to fetch.** `T` and `Option<T>` stop
  after two rows (enough to detect `TooManyRows`), and `exists()` stops after
  one.
- **`ExecError` lets callers inspect `ResultError`s uniformly.** Every driver
  error can carry one (`NotFound`, `TooManyRows`, decode failures), and
  `result_error()` returns it whatever the driver.
- **`Dialect` is an associated type.** Static capability bounds therefore read
  `where E::Dialect: Supports<caps::JsonPath>`.

### Executors chosen at run time

`run` is generic and the async methods return `impl Future`, so neither trait
is dyn-compatible. Type erasure is a separate layer instead:

- `DynExecutor` and `DynAsyncExecutor` are dyn-compatible, return boxed
  futures, and have blanket impls for every executor.
- `BoxExecutor` and `BoxAsyncExecutor` wrap a boxed executor and implement the
  normal traits again, with `Dialect = DynDialect` and `Error = DynError`.
- `DynError` keeps the original `ResultError` (if any) next to the boxed
  driver error.

So code written against `E: AsyncExecutor` works unchanged on a boxed
executor, but statically gated DSL functions don't compile against one
(`DynDialect` implements no `Supports<_>`). That's the runtime-gating path the
design calls for. `rupa-upwell` will build on `BoxAsyncExecutor` (or an `Arc`
variant once pools exist).

## Drivers

| crate | trait | notes |
|---|---|---|
| `rupa-driver-memory` | both | Evaluates the IR; never renders SQL. Has a new `Memory` dialect (`DialectId::Memory`, capabilities: `JsonPath`). |
| `rupa-driver-tokio-postgres` | `AsyncExecutor` | Wraps `tokio_postgres::Client`. |
| `rupa-driver-postgres` | `Executor` | Wraps `postgres::Client`. Reuses the async crate's `shared` module. |

### Postgres
- **Parameter types are inferred by the server** (no `prepare_typed`). Binding
  then adapts to the inferred type:
  - Integers narrow or widen when the value fits, so an `i64` parameter can
    compare against an `integer` column.
  - Typed nulls bind as SQL `NULL` to any type.
- **Reading follows the projection's `SqlType`,** with lossless integer and
  float widening (or narrowing, when the value fits) from the column's actual
  type.
- **Shared code:** `postgres::Row` and `postgres::Error` are re-exports from
  tokio-postgres. Value conversion, row cursors and `PgError` therefore live
  once, in `rupa_driver_tokio_postgres::shared`, and the sync driver depends
  on that crate. The alternative is a small `rupa-pg-common` crate, which
  needs one more published crate. It's easy to switch if you prefer that.

### Memory backend semantics
These match Postgres, and the conformance suite checks them:
- three-valued logic, including `NOT IN (.., NULL)`;
- `NULLS LAST` ascending and `NULLS FIRST` descending;
- `LIKE` with `%`, `_` and `\` escapes;
- `->` vs `->>` (JSON `null` is SQL `NULL` only under `->>`);
- primary-key uniqueness and `NOT NULL`;
- atomic multi-row `INSERT`, and `UPDATE` where `SET` sees the pre-update row.

Known differences:
- Text compares by byte order (Postgres' `C` collation), not by locale.
- Type errors appear when rows are evaluated, not when the query is planned.
- `->>` on an object or array is unsupported, because jsonb normalizes key
  order and spacing.
- Raw SQL, and DSL functions without `eval`, are `Unsupported`.

## Conformance suite (`rupa-conformance`, `publish = false`)

- 17 cases, written once against `AsyncExecutor`. Each runs on a freshly reset
  schema.
- Sync drivers run them through `Blocking<E>`, which completes immediately
  and still exercises the driver's own `Executor::execute`.
- Each driver's tests supply a `Harness`:
  - the memory harness re-registers tables;
  - the Postgres harnesses run `POSTGRES_DDL` against a `testcontainers`
    Postgres.
- Current runs:
  - memory: sync and async;
  - tokio-postgres: async;
  - postgres: sync via `Blocking`.

  All four pass, so memory and Postgres agree on every case.
- The suite includes a DSL function (`ci_eq`) that Postgres lowers to
  `upper(a) = upper(b)` and the memory backend evaluates through `eval`. It
  checks the deferred-lowering design from milestone 1 end to end.

## Follow-ups
- **Statement caching:** per query site, keyed by dialect, plus prepared
  statements on the driver side.
- **Transactions (M4):**
  - Generalize the Postgres executors over `GenericClient`, so a `Tx` is
    "just another executor".
  - Implement the async dirty-connection rollback strategy.
- **Pools:** not covered yet. They're needed for `rupa-upwell` (M9).
