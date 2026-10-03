# Milestone 6: DSL functions and `rupa-dsl-std`

Status: **implemented.**

## `#[dsl::function]`

```rust
use rupa::dsl::{self, Dialect, DialectId, DslError, Expr};

// Runtime-checked: the body branches on the dialect; unsupported ones get an error at render.
#[dsl::function(eval = eval_ilike)]
pub fn ilike(dialect: &dyn Dialect, s: Expr<String>, pattern: Expr<String>) -> Result<Expr<bool>, DslError> {
    match dialect.id() {
        DialectId::Postgres => Ok(Expr::raw_op("ILIKE", s, pattern)),
        DialectId::MySql | DialectId::Sqlite => Ok(Expr::raw_op("LIKE", Expr::call("LOWER", [s]), Expr::call("LOWER", [pattern]))),
        other => Err(DslError::unsupported("ilike", other)),
    }
}

// Statically gated: using it in a repository over a dialect without the capability does not compile.
#[dsl::function(requires = JsonContainment, eval = eval_contains)]
pub fn json_contains(dialect: &dyn Dialect, doc: Expr<Json>, part: Expr<Json>) -> Result<Expr<bool>, DslError> { .. }
```

- **The body is the lowering.** It runs at render time with the target
  dialect (`&dyn Dialect`, optional first parameter) and returns `Expr<R>` or
  `Result<Expr<R>, DslError>`.
- **No interpolation.** Parameters are `Expr<T>`, so data reaches SQL only as
  bound parameters. Operator and function names in `raw_op` / `call` are
  static text, validated by the renderer.
- **Options:**
  - `requires = Cap` / `[A, B]` gates statically;
  - `eval = path` (`fn(&[Value]) -> Result<Value, DslError>`) lets the memory
    backend run the function; without it, the memory backend returns
    `Unsupported`;
  - `name = ".."` sets the display name;
  - `crate = path` sets the crate path.
- **Generated under the function's name:**
  - `fn name(a: impl IntoExpr<T>, ..) -> Expr<R>`: builds the un-lowered call
    (`ExprNode::Dsl`). Usable from builder code, e.g. `ilike(col!(User::email), "a%")`.
  - `struct name {}`: a marker type. It implements `DslAvailable<D>` for every
    dialect, or, with `requires`, only for dialects with `Supports<Cap>`.
    Types and functions live in separate namespaces, so
    `use rupa::dsl::ilike` imports both.
- **In `#[query]`:** calls resolve by Rust path, e.g.
  `filter = dsl::ilike(email, $p) && longer_than(email, $n)`. Arguments can be
  columns (scalar or JSON), JSON paths, parameters, literals, or nested calls.

### Static vs runtime gating: what changed from the spec

The spec put static gating on "a capability bound on the function's generic
`D`". That can't work here. DSL calls stay un-lowered in the IR (needed for
the memory backend and for dialect-free queries), and lowering happens at
render time, where only `&dyn Dialect` exists, so a generic body has no `D` to
be instantiated with. Instead:

- **Declaring the gate.** `requires = Cap` declares it.
- **Where it's enforced at compile time.** `#[repository]` adds
  `fn_path: DslAvailable<<S as Source>::Dialect>` to the `Repo<S>` impl for
  each function its queries call. Over a `DynDialect` source, the
  `Arc<dyn Trait>` coercion fails with *"dialect `DynDialect` does not support
  `JsonContainment`"*.
- **Runtime re-check.** The lowering re-checks `dialect.supports(Cap)` and
  returns `DslError::Unsupported`. This covers builder code and free `#[query]`
  functions, which are dialect-free and so checked at render.

### Bounds on "methods": what Rust allows

You asked for capability bounds on methods rather than the trait. The trait
is indeed untouched, so mocks and other implementors never see the bounds.
The bound sits on the generated `Repo<S>` impl, though, not on each method.
Rust doesn't allow an impl method stricter bounds than its trait method
(E0276). Per-method bounds would therefore have to appear in the trait itself
(e.g. an associated `Dialect` type), which would leak into every
`dyn Trait<...>`. The practical effect: one repository trait that mixes a
statically gated function with others can't be implemented by `Repo<S>` for a
dialect lacking the capability. Use runtime-checked functions in repositories
meant for dynamically chosen dialects.

## `rupa-dsl-std`

These are written with the same macro, and nothing is special-cased:

| function | gating | Postgres | memory |
|---|---|---|---|
| `ilike` | runtime | `a ILIKE b` (MySQL/SQLite: `LOWER(a) LIKE LOWER(b)`) | eval |
| `lower`, `upper` | none | `lower(a)` | eval |
| `json_contains` | `JsonContainment` | `a @> b` | eval (jsonb semantics) |
| `json_has_key` | `JsonPath` | `jsonb_exists(a, k)` | eval |

Supporting changes:

- **Shared semantics.** `rupa_core::sem` holds the `LIKE` matcher and jsonb
  containment, used by the memory backend and by these `eval`s.
- **The `Memory` dialect declares `JsonContainment`**, since it can evaluate
  it.
- **JSON is an expression type** (`IntoExpr<serde_json::Value>`): JSON
  columns, JSON paths (`->`), `serde_json::Value` parameters, and
  `rupa::dsl::json(&value)`.
- **Connection sources (`Acquire` / `AcquireAsync`) expose their `Dialect`**,
  for the repository bounds.

## Tests

- **`rupa-dsl-std`:**
  - Postgres SQL snapshots for every function, including nested calls;
  - `ilike` lowered for each dialect id;
  - the static gate re-checked at lowering;
  - the `eval`s;
  - `sem` unit tests (LIKE, jsonb containment).
- **Conformance:** `case_dsl_std_text` and `case_dsl_std_json`, including the
  SQL-NULL behaviour. Memory (via `eval`) and Postgres (via lowering) agree.
- **`rupa-macros`:**
  - a user-defined function with `eval` in a repository, alongside the
    built-ins;
  - free `#[query]` rendering;
  - a runtime-checked function over a `BoxAsyncExecutor` (`DynDialect`);
  - 8 compile-error cases: the static gate over `DynDialect`, an unknown
    function, a wrong argument type, a generic, `async` or non-`Expr`
    function, a bad return type, and an unknown capability.
- **Tour:** `ci_eq` is now a `#[dsl::function]`, and the repository uses
  `ilike` and `json_contains`.
