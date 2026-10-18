# rupa-macros

## Role

Derive and attribute macros: `#[derive(Entity)]` and the capability derives,
`#[derive(Joined)]`, `#[query]`, `#[repository]`, `on!` and
`#[rupa::dsl::function]`.

## Usage

Through the `rupa` facade (`use rupa::prelude::*`). Generated code refers to
`::rupa::__macro_support`.

## Internal role

- `model` reads entity attributes; `entity`, `capability`, `joined` emit the
  derives.
- `dsl` parses the filter grammar shared by `#[query]` and `on!`.
- `query` turns a declared signature into a `Query<R>` builder expression
  (`attr` grammar, `ctx` filter codegen, `join`, `crud`, `raw` SQL).
- `repository` implements a trait for `Repo<S>` from its `#[query]` methods.

## Feature flags

| feature | adds |
|---|---|
| `async` | `async fn` methods in `#[repository]` (set by the facade's `async`) |
