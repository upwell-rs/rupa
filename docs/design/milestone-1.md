# Milestone 1: spike report, open decisions, design note

Status: **implemented.** All open questions in section 5 were accepted as proposed (2026-10-03).
License: `MIT OR Apache-2.0`. Releases: release-plz, with one `version_group` for all crates.

---

## 1. Spike: scalar-vs-JSON inference via autoref specialization

Code: `spikes/column-kind` (runtime) + `spikes/column-kind-macros` (derive).
`cargo +stable test -p rupa-spike-column-kind` passes on 1.99. Nightly 1.101
also passes, apart from the trybuild diagnostic wording.

### Mechanism

The derive emits `(&&&Probe::<Entity, FieldTy>::new("col")).__rupa_codec()` per
field. Method resolution picks the first level that applies:

| priority | applies when | result |
|---|---|---|
| 1 | `FieldTy: ScalarColumn` (incl. `Option<scalar>`) | `ScalarCodec<T>`, kind `Scalar` |
| 2 | `FieldTy = Option<U>`, `U: Serialize + DeserializeOwned` | `NullableJsonCodec<U>`, kind `Json`; `None` becomes SQL `NULL`, not JSON `null` |
| 3 | always, but the **method** requires `T: JsonColumn` | `JsonCodec<T>`, kind `Json` |

Level 3 puts its bound on the method, not the impl. That way a type that is
neither scalar nor serde gets our `on_unimplemented` message, pointing at the
field: *"`Opaque` cannot be stored in a column"*. Without this, rustc reports
"no method `__rupa_codec`".

### What holds up cleanly
- Concrete field types: `i64`, `String`, `DateTime<Utc>`, `Uuid`, `Vec<u8>`,
  `Option<_>`, serde structs, and `Vec<String>`. The `Vec<String>` case is JSON
  because Postgres arrays aren't in scope.
- `String` implements both `ScalarColumn` and serde. Scalar wins, as intended.
- `#[column(scalar)]` and `#[column(json)]` override inference by naming the
  codec directly, with no probe.
- **The kind is available at type level in expression context.**
  `col!(User::prefs)` has type `Column<User, Prefs, Json>`, so
  `.path("theme")` compiles. On a scalar column it fails with *"JSON path
  access requires a JSON column, but this column's kind is `Scalar`"*. That's
  enough for `#[query]` JSON paths, because all macro output is expressions.

### Limitations (none are blockers, but they shape the API)
1. **Inference can't happen in a type position.** The derive can't emit
   `pub const PREFS: Column<User, Prefs, ?>`. Typed column handles therefore
   come from an expression: `col!(User::prefs)`, or a prelude-trait variant
   decided in M3. Once you have a handle, you can still name its type by hand.
   For the same reason, `Entity::columns()` metadata is computed once into a
   `static` (a `OnceLock`) rather than being a `const`.
2. **Generic entity fields.** Dispatch resolves where the code is written, not
   where it is instantiated. A field `body: T` with `T: Serialize` bounds would
   resolve to JSON even for `Doc<i64>`; the test
   `dispatch_in_generic_context_follows_bounds` shows this. The spike's derive
   finds generic parameters in field types syntactically and **requires**
   `#[column(scalar|json)]` for those fields (see `tests/ui/generic_field.rs`).
3. **Semantic hazard: serde newtypes over scalars.** `struct Email(String)`
   with `#[serde(transparent)]` but no `ScalarColumn` compiles fine and becomes
   a JSON column, storing `"a@b.c"` as jsonb. This is the one place where
   inference picks a kind the user probably didn't mean. Mitigations:
   `#[derive(ScalarColumn)]` for newtypes (planned anyway), plus documentation.
   I can't see a sound compile-time lint for it.

**Verdict:** the spike holds up. I recommend keeping inference with the three
guardrails above. M1 doesn't depend on the derive, because it uses
hand-written `Entity` impls whose handles name their kind explicitly.

---

## 2. Decision needed: workspace versioning

| | Lockstep (one `workspace.package.version`) | Independent per crate |
|---|---|---|
| macros ↔ core coupling | natural: macros emit paths into `rupa_core::__private`, so they need an exact `=` pin anyway | still needs exact pins between macros and core, which makes those two lockstep in practice |
| users | one version number; `rupa = "0.x"` always gets a matching set | can upgrade a driver without upgrading core |
| releases | one tag, one changelog; unchanged crates get empty bumps | needs per-crate tooling (release-plz/cargo-release), changelogs, and a compatibility matrix |
| driver crates | an `Executor` break in core is a break for every driver regardless | only pays off once drivers are stable against a stable core |

**Recommendation: lockstep for everything until 1.0.** Revisit for drivers and
`rupa-upwell` after core 1.0. The scaffold already does this:
`version.workspace = true`, with intra-workspace deps pinned `=0.1.0` through
`[workspace.dependencies]`. Switching later is mechanical. Also unset so far:
`license`, which crates.io needs before publishing.

## 3. Decision needed: `rupa-sql` renderer

**sea-query**
- Pros: mature builders for PG, MySQL and SQLite; handles identifier quoting.
- Cons:
  - Has its own `Value` type, so every parameter is converted twice.
  - DSL lowering output (our IR) has to be mapped onto `SimpleExpr` and
    custom expressions, losing structure.
  - The placeholder/param-order model is sea-query's, not ours. That matters for
    typed nulls and the memory backend's parity.
  - It's a large 0.x dependency with frequent breaking releases, which ties up
    our MSRV and semver.
  - Snapshot tests would partly be testing sea-query.

**Hand-rolled**
- Pros:
  - The IR is small: select/insert/update/delete plus the expression tree.
  - Since values are **never** interpolated, the hard part sea-query solves
    (escaping literals) doesn't exist. Only identifier quoting remains, which is
    trivial.
  - We get full control of placeholders, param order, typed nulls and DSL
    lowering.
  - It's a single-digit-kLOC concern even with three dialects.
- Cons: we own correctness. Snapshots plus the conformance suite cover it.

**Recommendation: hand-rolled.**

---

## 4. Design note

### 4.1 Value
```rust
#[non_exhaustive]
pub enum Value {
    Null(SqlType),            // typed null: drivers need a type for unannotated params
    Bool(bool), I16(i16), I32(i32), I64(i64), F32(f32), F64(f64),
    Text(String), Bytes(Vec<u8>), Uuid(Uuid),
    Date(NaiveDate), Time(NaiveTime), Timestamp(NaiveDateTime), TimestampTz(DateTime<Utc>),
    Json(serde_json::Value),
}
```
`chrono`, `uuid` and `serde_json` are default features of `rupa-core`. A
`Decimal` variant would come behind a feature.

### 4.2 IR (untyped, `rupa_core::ir`)
```rust
pub enum Statement { Select(Select), Insert(Insert), Update(Update), Delete(Delete), Raw(RawSql) }

pub struct Select {
    pub from: From,                   // enum: From::Table(TableRef) now; From::Join(..) later
    pub projection: Vec<ColumnRef>,   // rows decode by position against this list
    pub filter: Option<ExprNode>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<ExprNode>,      // bound params too
    pub offset: Option<ExprNode>,
}
pub struct Insert { table: TableRef, columns: Vec<ColumnId>, rows: Vec<Vec<ExprNode>> }
pub struct Update { table: TableRef, set: Vec<(ColumnId, ExprNode)>, filter: Option<ExprNode> }
pub struct Delete { table: TableRef, filter: Option<ExprNode> }

pub enum ExprNode {
    Column(ColumnRef),                          // { source: SourceId, name, kind: ColumnKind }
    Param(Value),                               // the ONLY way a value enters the IR
    Unary(UnOp, Box<ExprNode>),                 // Not, IsNull, IsNotNull
    Binary(BinOp, Box<ExprNode>, Box<ExprNode>),// Eq Ne Lt Le Gt Ge And Or Like
    In(Box<ExprNode>, Vec<ExprNode>),
    JsonPath { column: ColumnRef, path: Vec<PathSeg> },
    Dsl(DslCall),                               // un-lowered DSL function call, see 4.4
    // Produced only by DSL lowering, never by users or the grammar directly:
    RawOp(&'static str, Box<ExprNode>, Box<ExprNode>),
    Call(&'static str, Vec<ExprNode>),
}
```
- `RawOp` and `Call` take `&'static str` names: operators and function names
  are author-controlled literals. Data can only reach SQL as `Param`.
- **Joins later:** `ColumnRef` carries a `SourceId` (table-alias index), not a
  table name. `From` is an enum, and projection is positional. Adding joins
  then adds a `From::Join` variant and a tuple `FromRow` decoder, without
  touching any existing node.
- **Composite ids later:** `Entity` exposes `ID_COLUMNS: &[ColumnId]` (a
  slice) and `Id: IdValues` (an id converts to `Vec<Value>`). v1 only
  implements `IdValues` for single scalars, but nothing assumes one id column.

Typed layer on top: `Expr<T>` wraps `ExprNode` and has a `PhantomData<T>`.
`Column<E, T, K>` gives `.eq(impl IntoExpr<T>)` etc., so a param's type is
checked against its column's `T` by the compiler. `bind(v)` makes an
`Expr<T>` from a `T: ToValue`.

### 4.3 How `Query<R>` carries its result type
```rust
pub struct Query<R> { stmt: Statement, expect: Expect, _r: PhantomData<fn() -> R> }
pub enum Expect { Many, AtMostOne, ExactlyOne, Affected, AnyAffected }
```
Two traits, so the result shape is tied to the statement kind:
- `FromRows`: implemented for `T: FromRow` (exactly one row; zero or more than
  one is an error), `Vec<T>` and `Option<T>`. Only `select(...)` accepts these.
- `FromAffected`: implemented for `u64` and `bool`. Only `insert`, `update` and
  `delete` accept these. A `RETURNING` variant can later accept `FromRows`.

`FromRow` is implemented for entities, scalars and tuples, but never for
`Vec`, `Option`, `u64` or `bool`. That keeps the blanket `impl<T: FromRow>
FromRows for T` coherent; I checked this on 1.99.

`Expect` comes from `R`'s associated const, so executors don't need to
dispatch on `R`. They call `R::from_output(output)` over a sans-IO `Row`
trait (`fn get(&self, idx) -> Result<Value, RowError>`, with the projection's
`ColumnKind` as the decode hint). For `ExactlyOne`/`AtMostOne`, the executor
fetches at most two rows.

### 4.4 Dialect capabilities

```rust
pub trait Dialect: Send + Sync + 'static {        // dyn-compatible
    fn id(&self) -> DialectId;
    fn supports(&self, cap: Capability) -> bool;  // runtime surface
    // rendering hooks: placeholder(n), quote_ident, ...
}
#[non_exhaustive] pub enum Capability { Ilike, JsonPath, JsonContainment, Returning, Savepoints, NativeRls, .. }

// Static surface: one ZST marker per capability, one generic marker trait.
pub trait CapabilityMarker { const CAP: Capability; }
#[diagnostic::on_unimplemented(message = "dialect `{Self}` does not support `{C}`")]
pub trait Supports<C: CapabilityMarker>: Dialect {}
```
- **Static gating:** an author writes `fn ilike<D: Supports<Ilike>>(..)`. Then
  `#[query]` output that uses it carries
  `where E: Executor<Dialect: Supports<Ilike>>`, and the error appears at
  compile time.
- **Runtime gating:** an author branches on `dialect.id()` or
  `dialect.supports(..)` and returns `Result`. `DynDialect`, the dialect chosen
  from config for `Box<dyn AsyncExecutor>`, implements `Dialect` but **no**
  `Supports<_>`. Statically gated functions are therefore unusable with it, and
  you have to use the runtime form. That's the "explicit, never implicit"
  requirement.
- **Keeping both in sync:** one macro per dialect, e.g.
  `capabilities!(Postgres: Ilike, JsonPath, ...)`, emits both the
  `Supports<_>` impls and the `supports()` match, so the two forms can't drift.

**Architectural consequence (please confirm):** DSL calls stay **un-lowered**
in the IR as `ExprNode::Dsl(DslCall { def: &'static DslFnDef, args })`, where
`DslFnDef` holds `name`, the `lower: fn(&dyn Dialect, Vec<ExprNode>) ->
Result<ExprNode, DslError>` (the user's function body), and an optional
`eval`. Lowering happens at render time, which gives three things:
1. The IR stays dialect-free, so one `Query<R>` can render to any dialect.
2. The memory backend sees `ilike(a, b)` and calls `eval`, instead of facing
   `LOWER(a) LIKE LOWER(b)` or a raw `ILIKE`. Without this, the memory backend
   couldn't work, and the IR would really be SQL in disguise.
3. Runtime-gated functions fail with `Unsupported` at render time, which is
   inside the `Result` every executor call already returns.

The static bound is still checked where the query is built.

### 4.5 Milestone 1 scope (once you approve)
`rupa-core`: `Value`, the IR, the typed builder, `Entity`, `Dialect`,
`Supports`, `Query<R>`, `FromRows` and `FromAffected`, all with hand-written
`Entity` impls in tests. `rupa-sql`: the Postgres renderer. Plus `insta`
snapshots for every IR node. `DslCall` lowering is wired into the renderer, but
`#[dsl::function]` is M6.

## 5. Open questions
1. Versioning: is lockstep OK? And which license?
2. Renderer: is hand-rolled OK?
3. DSL calls un-lowered in the IR, lowered at render time (4.4): OK?
4. What should `bool` mean as `R`? I propose *affected rows > 0* for DML. For an
   "exists" select, use `Query<bool>` via an explicit `.exists()` builder. Is
   that right?
5. Raw SQL (`#[query(sql = ..)]`): SQL text is inherently dialect-specific, so
   rule 5 can't hold for it. Should raw queries use one rupa placeholder syntax
   (`$name`) that we rewrite per dialect, and be documented as non-portable?
6. Should `Option<T>` mean "at most one" (an error if more than one row
   matches), or "first of possibly many"? I propose the former, fetching at
   most two rows.

---

## 6. Implementation notes (milestone 1 as built)

These are points where the code goes beyond, or narrows, the note above.

1. **JSON path keys are rendered as SQL literals, not bound.** Example:
   `"prefs" ->> 'theme'`. Keys are `&'static str` from source code, in the
   same category as identifiers. Binding them as parameters would stop
   Postgres expression indexes on `prefs ->> 'theme'` from ever matching.
   Quotes are doubled, keys containing a backslash use `E'..'`, and NUL is
   rejected. **Values are still always bound.**
2. **`Dialect` has no rendering hooks.** It is just `id()` and `supports()`.
   `rupa-sql` maps a `DialectId` to its own internal `Syntax`. This keeps core
   free of SQL text, but a third-party dialect can't be rendered without
   changing `rupa-sql`.
3. **`FromClause` and `SqlType` are deliberately exhaustive.** Adding a join
   or a SQL type must break every renderer and driver match, rather than
   falling into a wildcard arm.
4. **The typed builder doesn't check entity ownership inside expressions.**
   `Expr<T>` carries no entity parameter, so
   `select::<User>().filter(Other::COL.eq(..))` compiles. `update().set()`
   does check ownership, because its column is `Column<E, ..>`. `#[query]`
   resolves fields against the entity at macro time, so only hand-written
   builder code is affected. Joins will need source-typed expressions, and
   that's when to fix this.
5. **`set_expr` ignores nullability.** It can assign a nullable expression to a
   non-null column, and the database's `NOT NULL` constraint is what catches
   it. Comparisons ignore nullability on purpose, following SQL's
   three-valued logic.
6. **No single-scalar rows yet.** For example, `Query<Vec<i64>>` isn't
   supported. Implementing `FromRow` for scalars would overlap with the
   `Option<T>` result shape (`Option<i64>` is itself a scalar). A projection
   wrapper type will handle this when it's needed.
7. **`insert` writes every column, including the id.** Generated ids
   (`#[id(generated)]`, `RETURNING`) are part of the M3 `Insertable` design.
8. **`chrono` and `uuid` are default features of `rupa-core`.** The
   corresponding `Value` variants are cfg-gated.
9. **The spike stays in `spikes/` as a record.** The production version of the
   inference machinery lives in `rupa_core::column::__private`. The
   generic-field check moves into `rupa-macros` in M3.
10. **Rendered SQL hasn't been run against a real Postgres yet.** The
    snapshots were reviewed by hand. M2's container-based driver tests will
    execute them.
