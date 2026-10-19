# Migrations

Enable `migrations` and `sqlite` (bundled SQLite) or `sqlite-unbundled` (system
SQLite). Neither async nor runtime driver selection is required.

```sh
cargo run -p rupa --example sqlite_migrations --no-default-features --features migrations,sqlite
cargo run -p rupa --example sqlite_migrations --no-default-features --features migrations,sqlite -- app.db
```

The example uses an embedded SQL file to create notes, then a compiled Rust
migration reads typed `Note` rows, normalizes titles in Rust, and writes bound
updates. Passing a file makes history persist; a second invocation skips
installed work. The exact example catalog is also exercised by integration tests.

## Applying a catalog

```rust,ignore
let entries = application_migrations();
let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries)?;
let report = catalog.apply(SqliteExecutor::open("app.db")?)?;
```

`Catalog::new` validates versions, duplicates, source inputs and checksums without
opening a database or running a body. Entries are sorted numerically or by
validated UTC timestamp. `entries()` exposes metadata, checksum and exact static
operations for planned entries; imperative Rust bodies are explicitly opaque.

`apply` consumes a dedicated native connection and closes it after the run,
including failure and unwind. Use a separate connection for the runtime pool,
with separate credentials where supported. Do not wrap application transactions
or manually assemble runner/session handles. The driver's `Session` trait is a
control contract for adapters; ordinary application code uses `catalog.apply`.

The initial apply path is strict: transactional incremental migrations only.
Unsupported policy/kind is rejected before initialization. A nonempty unmanaged
target is refused; absence of a history table is not proof of freshness.
Changed or missing applied artifacts and late arrivals block pending work.
There is no silent reordering, checksum replacement or automatic body retry.

## Authoring

Use `sql_migration!` for a zero-sized type backed by an explicitly named SQL file,
then put `Planned<ThatType>` in `migration_catalog!`. Native SQLite parses scripts,
so quoted/comment semicolons are not split manually. Planned SQL is normalized
and lexically checked before the catalog can execute it.

A Rust module exports a type implementing `MigrationInfo` and `Migration<S>`.
Its static `Metadata.inputs` must embed the module and every local executable
helper/input with stable logical paths. Source and helper declarations are an
author responsibility, not transitive Rust dependency analysis. See
[`normalize.rs`](../crates/rupa/examples/migrations/normalize.rs) and
[the catalog](../crates/rupa/examples/migrations/mod.rs).

`Context::run` accepts ordinary typed Rupa queries. `Context::script` runs SQL on
the same transaction. Neither exposes transaction control, connection extraction
or pool access. SQLite authorization denies transaction/savepoint control,
ATTACH/DETACH, PRAGMAs and framework metadata access. This is an ownership API,
not a sandbox against trusted Rust independently doing IO or opening connections.

Checksums use `rupa-artifact-v1/sha256`: length-prefixed UTF-8 fields include the
stream, scheme, kind, version, sync mode, policy, execution style, ordered planned
SQL and sorted named input files. One leading UTF-8 BOM is removed and CRLF is
normalized to LF; all other bytes, including comments, remain significant. Lone
CR and noncanonical logical paths are rejected. Descriptions are diagnostic;
changing them alone does not change a checksum. Input names remain significant.
Facade features and the checkout's absolute path do not enter the digest.

This explicit catalog references individual files with `include_str!`, so Rust
tracks edits/removals. There is no directory discovery yet: additions require an
explicit catalog entry. Generated manifests, directory rebuild tracking, dialect
variants and crate-dependency closure records remain authoring work in #5.

## History and failure

SQLite reserves `_rupa_migration_meta`, `_rupa_migration_history`, and
`_rupa_migration_attempts`. A versioned schema signature, stream, scheme and
checksum format must match before managed execution. Incompatible same-named
objects are never replaced.

The driver atomically claims the target under `BEGIN IMMEDIATE`, then retains a
durable owner token between separately committed migration transactions. Another
cooperative runner cannot take over between commits or during first bootstrap.
The writer-lock timeout is configurable using
`SqliteExecutor::with_migration_lock_timeout`; an existing owner fails immediately.

Before dispatching a body, its attempt is durable. Schema/data effects, installed
history and attempt completion commit together. If a body fails, acknowledged
rollback records a failed attempt and releases the claim. Earlier completed
migrations remain installed. Errors preserve the failed version, typed body or
driver cause, and additional cleanup failures.

On interruption or an uncertain commit, a claim/running attempt can remain.
There is no safe automatic timeout takeover. Stop the old runner, inspect history
and database effects and perform explicit reconciliation before changing
ownership; a supported recovery command is a later slice. Do not delete history
or clear a claim merely because a client timeout elapsed. This first slice
reports uncertainty and blocks further apply instead of guessing.

Async and other-driver adapters, snapshots/adoption, authored down migrations,
nontransactional work, allowed late application and operator recovery are not yet
implemented. Their accepted design is in [the migration contract](design/migrations-contract.md).
