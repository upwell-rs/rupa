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
single-file Rust migration that normalizes titles through typed queries, a
directory migration with a private helper, and a planned migration that creates
a table from a shared entity. Passing a file makes history persist; a second
invocation skips installed work.

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
| `0002_normalize_titles.rs` | Scripted Rust migration; exports a unit struct `NormalizeTitles` implementing `Migration<S>` |
| `0003_add_slugs/mod.rs` (+ helpers) | Scripted Rust migration whose private helper files are part of its checksum |
| `0004_create_tags.rs` | Planned Rust migration; `CreateTags` implements `PlannedMigration` |

Leading zeros are dropped for numbered versions, so `1_a.sql` and `001_b.sql`
collide at build time. Leading `--` (SQL) or `//!` (Rust) lines are the
description; they are excluded from the hashed and executed body, so rewording
them never changes a checksum. Without a header, the name (`create notes`) is
used. Unrecognized entries fail the build; dotfiles are ignored. The build reruns
when the directory changes, so added or removed files never leave a stale catalog.

### Scripted and planned Rust migrations

A Rust migration is one of two kinds, decided by the trait its type
implements:

- **Scripted** (`Migration<S>`): imperative code with arbitrary steps between
  queries. It is opaque, so its checksum covers its source files.
- **Planned** (`PlannedMigration`): a pure `plan()` returning SQL scripts and
  [schema operations](#schema-operations). The plan is evaluated once when the
  catalog is built; its checksum covers the plan itself, and exactly that plan
  is executed.

A migration is frozen history, but application entities keep changing. If a
scripted migration 2 queried the crate's `Note` and migration 7 later added a
column to it, migration 2 would select a column that does not yet exist on
every fresh database, while its checksum still reported no change. `rupa-build`
therefore rejects scripted migrations that reach outside their own files:
`crate::` paths, `super::` above the migration root, `#[path]`, out-of-line
modules in single-file migrations, any `include*!` in a single-file migration,
and includes that leave a directory migration or name an unhashed dotfile.
Attribute arguments such as `#[derive(crate::Entity)]` are checked too. Declare
the row shape a scripted migration needs locally, as a private
`#[derive(rupa::Entity)]` struct, and keep helpers inside a directory migration.

Planned migrations may use shared entities (`Table::of::<crate::Tag>()`), so
they are not checked. Drift is caught instead of prevented: if an entity
change alters an applied migration's plan, its checksum no longer matches and
`apply` refuses to continue. Replace the entity reference with the literal
definition it used to produce (`Table::named`, `ColumnDef::new`); the same plan
keeps the same checksum, whatever source produced it.

External crates such as `rupa` stay available to both kinds; depending on a
shared crate is an explicit decision. `Context` exposes typed queries, scripts
and schema operations only, so repositories cannot run inside a migration.

## Schema operations

Schema operations are portable DDL in `rupa::migrate::schema`, rendered for the
session's dialect when the migration runs:

```rust,ignore
impl PlannedMigration for CreateTags {
    fn plan(&self) -> Plan {
        Plan::new()
            .create_table(Table::of::<Tag>())
            .create_table(
                Table::named("audit")
                    .column(ColumnDef::new("id", ColumnType::BigInt).identity())
                    .column(ColumnDef::new("at", ColumnType::TimestampTz)
                        .default(ColumnDefault::Sql("CURRENT_TIMESTAMP")))
                    .primary_key(["id"]),
            )
            .add_column(Note::SLUG)
            .create_index(Index::on("notes_slug", Note::SLUG).unique())
            .sql("UPDATE notes SET slug = lower(title)")
    }
}
```

`Table::of::<E>()` reads the entity's constant metadata: columns and types,
nullability, the primary key, `#[id(generated)]` identities and
`#[column(references = ..)]` foreign keys. Adjust it with `.column(..)` (add or
replace), `.without(..)` and `.foreign_key(..)`. `ColumnDef::of(Note::TITLE)`
builds one column from a field handle. Operations: create/drop/rename table,
add/drop/rename column, create/drop index.

Types render as the types each driver binds: for example `Json` is `JSONB`,
`JSON` and `TEXT`; `Uuid` is `UUID`, `CHAR(36)` and `TEXT`; identity keys are
`GENERATED BY DEFAULT AS IDENTITY`, `AUTO_INCREMENT` and
`INTEGER PRIMARY KEY AUTOINCREMENT`. `ColumnType::Custom` names a type per
dialect. Nothing is approximated; these fail before the target is touched:

- SQLite: adding a `NOT NULL` column without a default, or a `UNIQUE` column;
  an identity that is not the whole primary key; foreign keys into another
  attached database.
- MySQL: keys over unbounded `Text`, `Bytes` or `Json` (use `VarChar(n)`); text
  defaults containing a backslash; an identity that is not the whole primary key.
- Any dialect: a type without a name there, a database-generated column without
  a default, keys naming unknown columns.

`apply` renders every planned operation for the session's dialect before it
acquires the target. Scripted migrations run the same operations through
`Context::schema`. Changes SQLite can only make by rebuilding a table are not
offered; write them as SQL.

## Applying a catalog

```rust,ignore
let entries = application_migrations();
let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries)?;
let report = catalog.apply(SqliteExecutor::open("app.db")?)?;
```

`Catalog::new` validates versions, duplicates, source inputs and checksums without
opening a database or running a body. Entries are sorted numerically or by
validated UTC timestamp. `entries()` exposes metadata, checksum and the exact
plan of planned entries; scripted Rust bodies are explicitly opaque.

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
`MigrationInfo` and either `Migration<S>` or `PlannedMigration`. A scripted
migration's static `Metadata.inputs` must embed the module and every local
executable helper/input with stable logical paths; a planned one declares no
inputs, wrapped as `Planned<T>` in the catalog. Source and helper declarations
are then an author responsibility, not transitive Rust dependency analysis. The
facade's migration tests keep such an explicit catalog.

`Context::run` accepts ordinary typed Rupa queries. `Context::script` runs SQL on
the same transaction, and `Context::schema` a rendered schema operation. Neither exposes transaction control, connection extraction
or pool access. SQLite authorization denies transaction/savepoint control,
ATTACH/DETACH, PRAGMAs and framework metadata access. This is an ownership API,
not a sandbox against trusted Rust independently doing IO or opening connections.

Checksums use `rupa-artifact-v1/sha256`: length-prefixed UTF-8 fields include the
stream, scheme, kind, version, sync mode, policy, execution style, the ordered
plan (SQL and canonical schema operation records) and sorted named input files. One leading UTF-8 BOM is removed and CRLF is
normalized to LF; all other bytes, including comments, remain significant. Lone
CR and noncanonical logical paths are rejected. Descriptions are diagnostic;
changing them alone does not change a checksum. Input names remain significant.
Facade features and the checkout's absolute path do not enter the digest.

An explicit catalog references individual files with `include_str!`, so Rust
tracks edits and removals, but additions require a new catalog entry; directory
discovery with `rupa-build` avoids that. Dialect variants of one migration and
crate-dependency closure records remain later authoring work.

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
