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
Built-in scalar types written by name (`i64`, `String`, `Vec<u8>`, `Uuid`,
`DateTime<Utc>`, ..., and `Option` of them) get `Scalar` written directly, so
editors resolve their handles exactly; other types are selected by evaluating
a constant (`__RUPA_<FIELD>_ENCODING`). `Uuid` and the chrono types therefore
need rupa's `uuid`/`chrono` features (on by default); without them, such a
field fails as "not a scalar column type" unless marked `#[column(json)]`.

Repositories are dyn-compatible by default: async methods return boxed
futures, so the trait works as `Arc<dyn Trait>`, which DI frameworks inject.
`#[repository(static_dispatch)]` opts into native, unboxed futures for a trait
used only through its generated `Repo` type. Each trait gets one repository
type, `{Trait}Repo<C>`, over any `Connection` `C`; query methods take `&self`.
`repo.open_tx(options)` branches the repository into a transaction and
`repo.tx(&tx)` puts it on an open one. A dyn-compatible trait also gains `tx`,
returning a `RepoOn` that borrows the connection, so `Arc<dyn Trait>` runs on
a transaction without allocating a repository.
With the `upwell` feature, `#[repository(database = Primary)]` also registers
the repository as an Upwell provider over that declared database.

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
- `repository` generates the repository type over a connection from its `#[query]` methods, and its Upwell provider registration.

## Feature flags

| feature | adds |
|---|---|
| `async` | `async fn` methods in `#[repository]` (set by the facade's `async`) |
| `upwell` | `#[repository(database = ..)]` Upwell providers (set by the facade's `upwell-database`) |
