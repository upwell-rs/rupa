# Milestone 7: MySQL and SQLite

Status: **implemented**. Verified by the full conformance suite (23 general
and 12 transaction cases) against real MySQL 8 (Docker) and real SQLite.

## Dialects (`rupa_core::dialect`)

| capability | Postgres | MySQL 8.0+ | SQLite 3.38+ | Memory |
|---|---|---|---|---|
| `Ilike` | ✓ | | | |
| `JsonPath` | ✓ | ✓ | ✓ | ✓ |
| `JsonContainment` | ✓ | ✓ (`JSON_CONTAINS`) | | ✓ |
| `Returning` | ✓ | | ✓ | ✓ |
| `Savepoints` | ✓ | ✓ | ✓ | ✓ |
| `NativeRls` | ✓ | | | |
| `ReadOnlyTransactions` (new) | ✓ | ✓ | | ✓ |

Missing capabilities are errors, never silently ignored. Examples:

- `returning_one()` on MySQL is `RenderError::UnsupportedCapability(Returning)`.
- A read-only transaction on SQLite is refused at `begin`.
- `json_contains` in a repository over SQLite does not compile.

## Rendering to Postgres semantics

Postgres semantics are the reference: every backend must return the same
results for the same query. Where MySQL or SQLite behave differently, the
renderer writes SQL that reproduces Postgres' behaviour:

| difference | MySQL | SQLite |
|---|---|---|
| NULLs sort first ascending | `(x IS NULL) ASC, x ASC` (and `DESC, DESC`) | `x ASC NULLS LAST`, `x DESC NULLS FIRST` |
| `OFFSET` requires `LIMIT` | `LIMIT 18446744073709551615` | `LIMIT -1` |
| `->>` of JSON `null` is `'null'` | `CASE WHEN JSON_TYPE(..) = 'NULL' THEN NULL ELSE JSON_UNQUOTE(..) END` | (already SQL NULL) |
| no `CAST(.. AS BOOLEAN)` | `(x IN ('true', '1'))` | `CAST(x AS INTEGER)` |
| `LIKE` escape | `\` by default | `ESCAPE '\'` written |
| placeholders / identifiers | `?`, backticks | `?`, double quotes |
| `BEGIN` with options | `SET TRANSACTION ISOLATION LEVEL ..; START TRANSACTION [READ ONLY]` | `BEGIN` (any level is satisfied by SQLite's serializable transactions; read-only refused) |

JSON paths are `'$."key"[0]'`. Keys containing `"` or `\` are refused
(`InvalidJsonKey`) on MySQL and SQLite: how MySQL reads backslashes in string
literals depends on the server's SQL mode.

### What drivers and schemas must provide

Some differences can't be fixed in SQL:

- **MySQL text comparison and `LIKE` follow the column collation.** Use
  `utf8mb4_bin` (as the conformance schema does) for Postgres-like,
  case-sensitive results.
- **MySQL drivers must report matched rows for `UPDATE`.** That means setting
  `CLIENT_FOUND_ROWS`. Without it, `UPDATE` counts only changed rows.
- **SQLite `LIKE` is case-insensitive by default.** The rusqlite driver turns
  on `PRAGMA case_sensitive_like`.

## `rupa-driver-rusqlite` (pulled forward from milestone 11)

A sync `Executor` and `Transactional` on `rusqlite`. SQLite is bundled by
default; the `bundled` feature can be turned off.

- **Storage formats:** fixed-format text, so that text order is value order.
  - Timestamps are UTC `YYYY-MM-DDTHH:MM:SS.ffffffZ`.
  - Booleans are 0/1, UUIDs are hyphenated text, and JSON is text.
- **Reading** also accepts other common forms.
- **Transactions:** savepoints and the dirty-connection fallback, as in the
  Postgres drivers. `From<SqliteError> for DynError` is provided.
- **No async runtime needed.** Async repositories work over it through
  `Repo::shared(..)`, as discussed for sync-only apps.

## MySQL verification without a driver

A test-only executor (`crates/rupa-conformance/tests/mysql.rs`) runs on the
pure-Rust `mysql` crate and runs the whole suite against a MySQL 8 container.
It does what the future sqlx driver must do:

- bind and read values;
- set `CLIENT_FOUND_ROWS`;
- run transaction statements one at a time.

## Conformance changes

Cases now ask the dialect what it supports:

- **No `RETURNING`:** generated values are read back with a query, and
  `returning_one()` is checked to fail.
- **No `JsonContainment`:** those assertions are skipped.
- **No read-only transactions:** `begin_with(read_only)` must be refused.

The suite's own test function (`ci_eq`) is now a `#[dsl::function]` lowered
for every SQL dialect. On its first SQLite run, the old Postgres-only version
was correctly refused, which showed the runtime gate working.

`rupa-dsl-std` gains MySQL and SQLite forms:

- `ilike` becomes `LOWER .. LIKE LOWER ..` as a real `LIKE` node, so SQLite's
  `ESCAPE` applies;
- `json_contains` uses MySQL's `JSON_CONTAINS`;
- `json_has_key` uses MySQL's `JSON_CONTAINS_PATH` and SQLite's `json_type`.
  On SQLite, a SQL-NULL document gives `false` rather than `NULL`. That's only
  visible under `NOT`.
