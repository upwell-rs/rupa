# rupa-sql

## Role

Renders `rupa-core` IR to SQL text plus bound parameters for Postgres, MySQL
and SQLite. Values are never interpolated.

## Usage

Drivers call `render_cached(statement, dialect)` and send the SQL with its
parameters; `render_tx` renders transaction control.

## Internal role

- `writer` is the dialect-neutral renderer, parameterized by a `Syntax` per
  dialect (`postgres`, `mysql`, `sqlite`).
- `cache` caches rendered SQL by statement shape (never by value), in a
  sharded LRU (`hashlink`). See `docs/design/caching.md`.
- `driver_error!` declares the error enum every SQL driver shares.

## Feature flags

None.
