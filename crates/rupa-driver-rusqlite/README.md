# rupa-driver-rusqlite

## Role

Sync SQLite (3.38+) executor and pool on rusqlite and r2d2.

## Usage

`SqlitePool::connect(&DbConfig::new("sqlite://app.db"))?`, then bind a generated repository view to `&pool`. Usually enabled through a feature of the `rupa` facade.

## Internal role

Values are bound without copying text or blobs; dates use `rupa_core::chrono_text`, shared with sqlx-sqlite. The pool is the core's `r2d2_pool!`.

## Feature flags

| feature | adds |
|---|---|
| `bundled` (default) | compiles SQLite from source |
| `chrono`, `uuid` (default) | date/time and UUID columns |
| `pool` | `SqlitePool` on r2d2 |
| `dynamic` | `SqliteDriver`, the registry entry (implies `pool`) |

`migrations` implements the standalone migration session contract on the native executor. Use `catalog.apply(connection)`; no additional session/runner handle is required. Connections are dedicated and owned for the invocation. See [the migration guide](../../docs/migrations.md).
