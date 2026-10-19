# Migrations

Enable `migrations` and a supported driver: `sqlite` (bundled SQLite),
`sqlite-unbundled` (system SQLite), `postgres-sync` (the sync `postgres`
crate) or `postgres` (async, tokio-postgres). Sync drivers need neither async
nor runtime driver selection.

## Execution modes

Execution comes in two modes with identical names, selected by import, as
with Diesel's sync and async query traits: `rupa::migrate::sync` (feature
`sync`) and `rupa::migrate::asynchronous` (feature `async`). Each holds
`Session`, `Migration`, `Context` and `Apply`; catalogs, plans, metadata,
errors and reports are shared at `rupa::migrate`. Neither mode is a default:
sync-only drivers (`sqlite`, `sqlite-unbundled`, `postgres-sync`, `mysql`)
enable `sync`, async drivers enable `async`, and a crate that only owns
migrations enables the mode its Rust bodies are written for.

```rust,ignore
use rupa::migrate::sync::Apply;

let report = catalog.apply(SqliteExecutor::open("app.db")?)?;
```

```rust,ignore
use rupa::migrate::asynchronous::Apply;

let report = catalog.apply(connection).await?;
```

With exactly one mode enabled, its four names are also re-exported at
`rupa::migrate` and the facade prelude brings `Apply` and `Session` into scope.
With both enabled, neither is re-exported, so the two `apply` methods can never
be ambiguous; import from the mode module. Because Cargo unifies features
across a build, a dependency that enables the other mode removes the root
names: libraries, and code built in workspaces that enable both, should import
from the mode modules.

Planned and SQL migrations (`Planned<T>`) and `migration_catalog!` catalogs
implement every enabled mode. A scripted Rust body implements the mode it is
written for; implementing both is the same body with `.await` added.
`Blocking(connection)` runs any sync session through the async contracts with
no added locking, for async applications on sync-only drivers.

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
use rupa::migrate::sync::Apply;
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

- **Scripted** (`sync::Migration<S>` or `asynchronous::Migration<S>`):
  imperative code with arbitrary steps between queries. It is opaque, so its checksum covers its source files.
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
- MySQL: primary keys or `UNIQUE` columns over unbounded `Text`, `Bytes` or
  `Json` (use `VarChar(n)`); text defaults containing a backslash; an identity
  that is not the whole primary key.
- Any dialect: a type without a name there, a database-generated column without
  a default, `CREATE TABLE` keys naming unknown columns, an empty plan.

Indexes and foreign keys name columns without their types, so a MySQL index
over unbounded text or an index naming an unknown column is rejected by the
server when the migration runs.

`apply` renders every planned operation for the session's dialect before it
acquires the target. Scripted migrations run the same operations through
`Context::schema`. Changes SQLite can only make by rebuilding a table are not
offered; write them as SQL.

## Applying a catalog

```rust,ignore
use rupa::migrate::sync::Apply;

let entries = application_migrations();
let catalog = Catalog::new("primary", VersionScheme::Numbered, &entries)?;
let report = catalog.apply(SqliteExecutor::open("app.db")?)?;

let client = postgres::Client::connect(url, postgres::NoTls)?;
let report = catalog.apply(PgExecutor::new(client))?;
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
Sync and async runs share one runner body, so history, exclusion and failure
handling are identical in both modes.

A run that ends without returning (a dropped `apply` future, or a panic
unwinding through it) abandons its session: the runner's drop guard calls the
non-blocking `Session::abandon`, and a retained session then refuses every
statement and run with `StateError::Poisoned`. A consumed connection is simply
closed, which on PostgreSQL rolls back the open transaction and releases the
lock; the journaled attempt stays running and blocks the next apply until it is
reconciled. Nothing an abandoned body did is committed.

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
the session dialect's grammar before acquiring the target. SQLite and
PostgreSQL have script grammars; other dialects reject scripts explicitly
(`ScriptError::Dialect`).

The PostgreSQL grammar assumes `standard_conforming_strings` (the migration
connection must have it on) and keeps standard and `E''` strings, quoted
identifiers, nested block comments and dollar-quoted bodies whole. PostgreSQL
has no client-side authorizer, so statements the runner owns are refused before
dispatch (`ScriptError::Forbidden` with a `ScriptRule`): transaction control,
session-level `SET`, `set_config`, `RESET`, `DISCARD`, `LISTEN` and `UNLISTEN`
(`SET LOCAL`, `SET CONSTRAINTS` and `SET TRANSACTION` are allowed), psql
backslash commands and `COPY` from stdin or to stdout,
advisory lock functions, `_rupa_`-prefixed names and `BEGIN ATOMIC` bodies (use a
dollar-quoted body instead). Typed raw queries are checked by the same grammar.
Dollar-quoted bodies are opaque to it; the session's lock and transaction checks
below catch what they do to the runner.

Without `rupa-build`, a hand-written Rust module exports a type implementing
`MigrationInfo` and either a mode's `Migration<S>` or `PlannedMigration`. A scripted
migration's static `Metadata.inputs` must embed the module and every local
executable helper/input with stable logical paths; a planned one declares no
inputs, wrapped as `Planned<T>` in the catalog. Source and helper declarations
are then an author responsibility, not transitive Rust dependency analysis. The
shared migration conformance fixtures (`rupa-conformance`) keep such an explicit
catalog.

`Context::run` accepts ordinary typed Rupa queries. `Context::script` runs SQL on
the same transaction, and `Context::schema` a rendered schema operation. Neither exposes transaction control, connection extraction
or pool access. SQLite authorization denies transaction/savepoint control,
ATTACH/DETACH, PRAGMAs and framework metadata access; PostgreSQL uses the grammar
above. This is an ownership API,
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

Both engines reserve `_rupa_migration_meta`, `_rupa_migration_history`, and
`_rupa_migration_attempts`; on PostgreSQL they live in the connection's current
schema, the owned namespace. A versioned schema signature, stream, scheme and
checksum format must match before managed execution. Incompatible same-named
objects are never replaced.

On SQLite the driver atomically claims the target under `BEGIN IMMEDIATE`, then retains a
durable owner token between separately committed migration transactions. Another
cooperative runner cannot take over between commits or during first bootstrap.
The writer-lock timeout is configurable using
`SqliteExecutor::with_migration_lock_timeout`; an existing owner fails immediately.

On PostgreSQL, the sync (`postgres`) and async (`tokio-postgres`) drivers run
one shared session protocol, so they exclude each other and continue each
other's history. `apply` takes a session advisory lock before creating or
reading history (standalone `Session::history()` inspection reads without it),
holds it across the separately committed migration
transactions and checks it before every write phase. The key is derived from
the canonical target (`rupa_migrate::Target`: engine, `current_database()`,
`current_schema()`), never from the driver, credentials or URL, so every runner
of one schema contends for the same lock and different schemas are independent
targets. `PgExecutor::with_migration_lock_timeout` bounds the wait (zero tries
once; longer than PostgreSQL's `lock_timeout` maximum is clamped); a timeout
reports `Error::Busy` with no user work done. A connection that
loses the lock, or whose artifact transaction was committed or replaced, is
poisoned instead of recording completion. Closing the connection releases the
lock, so a dropped or crashed runner leaves only its running attempt behind,
which blocks the next apply with `Error::Uncertain`. A connection already inside
an application transaction is rejected.

Before dispatching a body, its attempt is durable. Schema/data effects, installed
history and attempt completion commit together. If a body fails, acknowledged
rollback records a failed attempt and releases the claim. Earlier completed
migrations remain installed. Errors preserve the failed version, typed body or
driver cause, the versions this invocation already committed (`applied()`), and
additional cleanup failures. Misusing a migration connection outside a
runner-owned transaction fails with a typed `StateError`
(`SqliteError::Migration`, `PgError::Migration`).

On interruption or an uncertain commit, a claim/running attempt can remain.
There is no safe automatic timeout takeover. Stop the old runner, inspect history
and database effects and perform explicit reconciliation before changing
ownership; a supported recovery command is a later slice. Do not delete history
or clear a claim merely because a client timeout elapsed. This first slice
reports uncertainty and blocks further apply instead of guessing.

sqlx sessions, MySQL, snapshots/adoption, authored down
migrations, nontransactional work (such as `CREATE INDEX CONCURRENTLY`), allowed
late application and operator recovery are not yet implemented. Their accepted
design is in [the migration contract](design/migrations-contract.md).
