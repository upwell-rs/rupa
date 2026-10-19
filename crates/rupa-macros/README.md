# rupa-macros

## Role

Derive and attribute macros: `#[derive(Entity)]` and the capability derives,
`#[derive(Joined)]`, `#[query]`, `#[repository]`, `on!` and
`#[rupa::dsl::function]`.

## Usage

Through the `rupa` facade (`use rupa::prelude::*`). Generated code refers to
`::rupa::__macro_support`.

`#[derive(Entity)]` emits one typed column handle constant per field, named
in uppercase (`created_at` becomes `User::CREATED_AT`), and the constant
`Entity::COLUMNS` metadata. A field is a scalar column when its type implements
`ScalarColumn`, a nullable JSON column when it is an `Option` of a serde type,
and a JSON column for any other `Serialize + DeserializeOwned` type; the
choice is made at compile time, and `#[column(scalar | json)]` overrides it.

Repositories use native, unboxed futures by default. Use
`#[repository(dynamic_dispatch)]` when the trait must support `dyn Trait`.
`&mut self` query methods borrow a connection or transaction directly;
`&self` methods acquire from a pool or an explicitly shared context.

Raw `#[query(sql = "...")]` statements use `$name` parameters outside quotes
and comments. Backslashes immediately before a closing or escaped quote are
rejected because their meaning depends on the database and session settings.
Bind such values as parameters, or use SQL's doubled quotes for literal quotes.

## Internal role

- `model` reads entity attributes; `entity`, `capability`, `joined` emit the
  derives.
- `dsl` parses the filter grammar shared by `#[query]` and `on!`.
- `query` turns a declared signature into a `Query<R>` builder expression
  (`attr` grammar, `ctx` filter codegen, `join`, `crud`, `raw` SQL).
- `repository` generates a distinct owned repository and borrowed view from its `#[query]` methods.

## Feature flags

| feature | adds |
|---|---|
| `async` | `async fn` methods in `#[repository]` (set by the facade's `async`) |
