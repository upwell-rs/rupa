# Migrations

Enable `migrations` and `sqlite` (bundled SQLite) or `sqlite-unbundled` (system
SQLite). Neither async nor runtime driver selection is required.

```sh
cargo run -p rupa-example-app
cargo run -p rupa-example-app -- app.db
```

The example in [`examples/migrations`](../examples/migrations) is split like an
application: `db` owns entities and a `migrations/` directory, and `app`
applies the generated catalog on startup. The directory holds a SQL file, a
single-file Rust migration that normalizes titles through typed queries, and a
directory migration with a private helper. Passing a file makes history
persist; a second invocation skips installed work.

## Discovering a directory

`rupa-build` generates the catalog from file names at build time, so adding a
migration means adding a file:

```rust,ignore
// db/build.rs
fn main() -> Result<(), rupa_build::Error> {
    rupa_build::Migrations::new("primary").compile()
}

// db/src/lib.rs
pub mod migrations {
    rupa::migrate::include_migrations!("primary");
}

// app: the catalog is engine-independent and can be applied from any crate
db::migrations::catalog()?.apply(SqliteExecutor::open("app.db")?)?;
```

| Entry | Meaning |
|---|---|
| `0001_create_notes.sql` | Planned SQL migration, version `1` |
| `0002_normalize_titles.rs` | Rust migration; exports a unit struct `NormalizeTitles` implementing `Migration<S>` |
| `0003_add_slugs/mod.rs` (+ helpers) | Rust migration whose private helper files are part of its checksum |

Leading zeros are dropped for numbered versions, so `1_a.sql` and `001_b.sql`
collide at build time. Leading `--` (SQL) or `//!` (Rust) lines are the
description; they are excluded from the hashed and executed body, so rewording
them never changes a checksum. Without a header, the name (`create notes`) is
used. Unrecognized entries fail the build; dotfiles are ignored. The build reruns
when the directory changes, so added or removed files never leave a stale catalog.

### Rust migrations are self-contained

A migration is frozen history, but application entities keep changing. If
migration 2 queried the crate's `Note` and migration 7 later added a column to
it, migration 2 would select a column that does not yet exist on every fresh
database, while its checksum still reported no change. `rupa-build` therefore
rejects Rust migrations that reach outside their own files: `crate::` paths,
`super::` above the migration root, `#[path]`, out-of-line modules in
single-file migrations and `include*!` outside the migration directory.

Declare the row shape a migration needs locally, as a private
`#[derive(rupa::Entity)]` struct, and keep helpers inside a directory migration.
External crates such as `rupa` stay available; depending on a shared crate is an
explicit decision whose drift the checksum cannot see. `Context` exposes typed
queries and scripts only, so repositories cannot run inside a migration.

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
and checksummed independently of any dialect; `apply` lexically checks it with
the session dialect's grammar before acquiring the target. Only SQLite has a
script grammar so far; other dialects reject scripts explicitly.

Without `rupa-build`, a hand-written Rust module exports a type implementing
`MigrationInfo` and `Migration<S>`. Its static `Metadata.inputs` must embed the
module and every local executable helper/input with stable logical paths. Source
and helper declarations are then an author responsibility, not transitive Rust
dependency analysis. The facade's migration tests keep such an explicit catalog.

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
driver cause, the versions this invocation already committed (`applied()`), and
additional cleanup failures. Misusing a SQLite migration connection outside a
runner-owned transaction fails with `SqliteError::Migration`.

On interruption or an uncertain commit, a claim/running attempt can remain.
There is no safe automatic timeout takeover. Stop the old runner, inspect history
and database effects and perform explicit reconciliation before changing
ownership; a supported recovery command is a later slice. Do not delete history
or clear a claim merely because a client timeout elapsed. This first slice
reports uncertainty and blocks further apply instead of guessing.

Async and other-driver adapters, snapshots/adoption, authored down migrations,
nontransactional work, allowed late application and operator recovery are not yet
implemented. Their accepted design is in [the migration contract](design/migrations-contract.md).
