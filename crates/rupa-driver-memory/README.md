# rupa-driver-memory

## Role

In-memory backend: evaluates the IR directly, never renders SQL. For tests and prototypes.

## Usage

`MemoryDb` (sync executor) or `db.into_async()` (`AsyncMemoryDb`); register tables with `db.register::<User>()`. Usually enabled through a feature of the `rupa` facade.

## Internal role

`eval` evaluates expressions with Postgres semantics; `table` holds rows and row scopes; transactions snapshot the tables.

## Feature flags

| feature | adds |
|---|---|
| `chrono`, `uuid` (default) | date/time and UUID values |
| `async` | `AsyncMemoryDb` |
| `dynamic` | `MemoryDriver`, the registry entry (implies `async`) |
