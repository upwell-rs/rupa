# Migrations

Enable `migrations` and a supported driver: `sqlite` (bundled SQLite),
`sqlite-unbundled` (system SQLite), `postgres-sync` (the sync `postgres`
crate), `mysql` (the sync `mysql` crate), `postgres` (async, tokio-postgres) or
an async sqlx driver (`sqlx-postgres`, `sqlx-mysql`, `sqlx-sqlite`,
`sqlx-sqlite-unbundled`). Sync drivers need neither async nor runtime driver
selection.

Every driver of an engine runs that engine's one session protocol
(`rupa_migrate::protocol`, which drivers implement through a small IO trait):
the same lock identity, reserved tables, checks and policies. So a native and
a sqlx runner on one database exclude each other and continue each other's
history, in either mode.

## Execution modes

Execution comes in two modes with identical names, selected by import, as
with Diesel's sync and async query traits: `rupa::migrate::sync` (feature
`sync`) and `rupa::migrate::asynchronous` (feature `async`). Each holds
`Session`, `Migration`, `Context`, `Apply`, `Inspect` and `Recover`; catalogs, plans, metadata,
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

With exactly one mode enabled, its six names are also re-exported at
`rupa::migrate` and the facade prelude brings `Apply`, `Inspect`, `Recover` and
`Session` into scope.
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
used.

From its first line starting with `rupa:`, the header is YAML frontmatter
instead of description, with the comment markers removed:

```sql
-- Index titles without blocking writes.
-- rupa:
--   policy: nontransactional
```

The flow form `-- rupa: { policy: nontransactional }` is the same document.
`policy` is `transactional` (the default) or `nontransactional` (see
[nontransactional migrations](#nontransactional-migrations)), and is part of
the checksum. Unknown keys or values and malformed YAML fail the build with the
parser's location. Only the migration root (its `.sql` file, `.rs` file or
`mod.rs`) declares frontmatter: a scripted directory migration's `.rs` helper
with a `rupa:` header fails the build, and a planned directory migration's
helpers and non-Rust helpers are never header-parsed. Unrecognized entries
fail the build; dotfiles are ignored. The build reruns
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

let report = catalog.apply(MySqlExecutor::connect(mysql::Opts::from_url(url)?)?)?;
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
statement with `StateError::Poisoned` (and a new run with `Error::History`).
Pools discard a connection a run left in this or any other unfinished state,
instead of handing it out again. A consumed connection is simply
closed, which on PostgreSQL rolls back the open transaction and releases the
lock; the journaled attempt stays running and blocks the next apply until it is
reconciled. Nothing an abandoned transactional body did is committed.

The apply path is strict: incremental migrations only, with the policies the
session supports (`Session::POLICIES`). An unsupported policy or kind is
rejected before initialization and is never downgraded: SQLite runs
`TransactionalRequired` only, PostgreSQL also `NonTransactional`, and MySQL
`NonTransactional` only, because it commits DDL implicitly. A nonempty unmanaged
target is refused; absence of a history table is not proof of freshness.
Changed or missing applied artifacts and late arrivals block pending work.
There is no silent reordering, checksum replacement or automatic body retry.

## Inspecting a target

Three read-only checks answer what `apply` would do before anyone runs it.
None of them takes exclusion, creates a journal, writes or executes a body.

```rust,ignore
use rupa::migrate::{Engine, sync::Inspect};

// Offline, in CI: the catalog against what an engine's sessions can execute.
catalog.validate(Engine::Postgres)?;

// The catalog against a target.
let status = catalog.status(connect()?)?;

// What apply would run there: the status and every pending step.
let plan = catalog.plan(connect()?)?;
```

`validate` runs the checks `apply` runs before it touches a target: supported
kinds and policies, planned scripts that pass the engine's grammar, and schema
operations that render for it.

`status` classifies the target as `Fresh`, `Managed`, `Unmanaged` or
`Uncertain` (running attempts or an owner claim). It returns the installed
history with each entry's provenance, the running attempts, the pending catalog
entries, and every `Blocker`, each with the version, checksum or attempt it
concerns:

| blocker | apply would |
|---|---|
| `Unmanaged` | refuse the target until it is adopted |
| `Stream` | refuse a journal of another stream or version scheme |
| `Locked` | report `Error::Busy`: a live runner holds the PostgreSQL or MySQL lock |
| `Claimed` | report `Error::Busy` until SQLite's owner claim is released |
| `Interrupted(attempt)` | record it rolled back and stop with `Error::Reconciled` |
| `Unresolved(attempt)` | stop with `Error::Uncertain` until it is [recovered](#recovery) |
| `Missing`, `Changed`, `Duplicate`, `Late` | refuse history that no longer matches the catalog |

`apply` runs the pending entries only when there are no blockers. While a
runner is live, its running attempts are listed but not reported as
blockers, since they may be in progress.

`plan` returns the status and one `Step` per pending entry, in the order
`apply` runs them: the entry's version, checksum and policy, and its
`Execution`. A planned body is `Static`, with its exact scripts rendered
for the session's dialect. A Rust body is `Opaque` until it runs.

The engines read inside a snapshot the server refuses to write through
(`READ ONLY` on PostgreSQL and MySQL; a deferred read transaction on SQLite).
They observe exclusion without taking it: the advisory lock in `pg_locks`,
`IS_USED_LOCK` on MySQL, and the claim column on SQLite. They never wait for
exclusion. On SQLite in rollback-journal mode, a read waits, up to the
connection's `busy_timeout`, while a writer holds its exclusive lock to
commit, and fails if the timeout runs out. WAL readers never wait. A status
is a snapshot, and a runner may start right after it was read. `apply`
re-checks everything under exclusion.

## Authoring

Use `sql_migration!` for a zero-sized type backed by an explicitly named SQL file,
then put `Planned<ThatType>` in `migration_catalog!`. An optional
`policy: NonTransactional,` field (before `up`) declares nontransactional work;
it defaults to `TransactionalRequired`. Native SQLite parses scripts,
so quoted/comment semicolons are not split manually. Planned SQL is normalized
and checksummed independently of any dialect; `apply` lexically checks it with
the session dialect's grammar before acquiring the target. SQLite and
PostgreSQL and MySQL have script grammars; other dialects reject scripts
explicitly (`ScriptError::Dialect`).

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

The MySQL grammar assumes `NO_BACKSLASH_ESCAPES` without `ANSI_QUOTES`: for
the run, the migration session adds the first to its `sql_mode` and removes the
second (and the composite `ANSI`), restoring the original at release. So a
backslash in a string is literal and `"…"` is always a string, never an
identifier that could name a reserved table. It keeps `'…'` and `"…"` strings, backtick identifiers, `#`
and `-- ` line comments (`--` without following whitespace is two minus signs)
and `/* */` comments whole. It refuses before dispatch: executable comments
(`/*! … */`), transaction control including `XA` and `LOCK`/`UNLOCK TABLES`,
every `SET` and `USE`, the client's `DELIMITER`, `SOURCE` and `LOAD DATA LOCAL`,
named lock functions, `_rupa_`-prefixed names, and `BEGIN … END` compound
statements, whose boundaries it cannot know (use single-statement routines).
Statement text built at run time (`PREPARE … FROM '…'`) is opaque, like a
PostgreSQL dollar-quoted body.

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
ATTACH/DETACH, PRAGMAs and framework metadata access, with one policy for every
SQLite driver: rusqlite installs it through its authorizer hook, sqlx (which has
none) through the raw connection handle. PostgreSQL and MySQL use the grammar
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

Every engine keeps one reserved table, the journal `_rupa_migrations`. On
PostgreSQL it lives in the connection's current schema, on MySQL as an InnoDB
table in the selected database (the owned namespace), and on SQLite in `main`.
Its definition per engine, with every row kind and status explained, is in
[`crates/rupa-migrate/sql/`](../crates/rupa-migrate/sql/): `postgres.sql`,
`mysql.sql` and `sqlite.sql`. These files are what the runner executes on a
fresh target, so they always describe the real table.

One `init` row records the stream, version scheme and checksum format. Each
attempt is an `up` row, journaled `running` before its body starts and finished
as `applied`, `failed`, `rolled_back`, `recovered` or `abandoned`. Installed
history is the applied and recovered rows; an installation rank is the row's
id, so ranks increase in installation order but are not consecutive. The
journal's columns, the header and the stream, scheme and checksum format must
match before managed execution. Incompatible same-named objects are never
replaced.

On SQLite the driver atomically claims the target under `BEGIN IMMEDIATE`, then retains a
durable owner token between separately committed migration transactions. Another
cooperative runner cannot take over between commits or during first bootstrap.
The writer-lock timeout is configurable using
`with_migration_lock_timeout` on the rusqlite or sqlx executor; an existing
owner fails immediately.

On PostgreSQL (`postgres`, `tokio-postgres` and sqlx), `apply` takes a session advisory lock before creating or
reading history (standalone `Session::history()` inspection reads without it),
holds it across the separately committed migration
transactions and checks it before every write phase. The key is derived from
the canonical target (`rupa_migrate::Target`: engine, `current_database()`,
`current_schema()`), never from the driver, credentials or URL, so every runner
of one schema contends for the same lock and different schemas are independent
targets. `with_migration_lock_timeout` bounds the wait (zero tries
once; longer than PostgreSQL's `lock_timeout` maximum is clamped); a timeout
reports `Error::Busy` with no user work done. A connection that
loses the lock, or whose artifact transaction was committed or replaced, is
poisoned instead of recording completion. Closing the connection releases the
lock, so a dropped or crashed runner leaves only its running attempt behind
(see [Interruption](#interruption)). A connection already inside an application
transaction is rejected.

On MySQL (the `mysql` and sqlx drivers), `apply` takes a named session
lock, `GET_LOCK('rupa:…')` with the first 56 hex digits of the canonical target
digest (engine, `DATABASE()` as database and namespace), and keeps it across the
per-artifact commits, checking `IS_USED_LOCK` before every write phase.
`with_migration_lock_timeout` bounds the wait in whole seconds,
rounded up; zero tries once. A connection without a selected database, with
`autocommit` off or inside an open transaction (even an untouched read-only
snapshot) is rejected: MySQL's implicit DDL commits would otherwise commit the
caller's work. Creating the journal commits on its own, before its `init` row
is written, so an interrupted first run can leave a journal without a header,
which later runs refuse as invalid rather than complete.

Before dispatching a body, its attempt is durable. A transactional body's
schema/data effects and its attempt's completion commit together.
If a transactional body fails,
acknowledged rollback records a failed attempt and releases the claim. Earlier completed
migrations remain installed. Errors preserve the failed version, typed body or
driver cause, the versions this invocation already committed (`applied()`), and
additional cleanup failures. Misusing a migration connection outside a
runner-owned transaction or body fails with a typed `StateError`
(`SqliteError::Migration`, `PgError::Migration`, `MySqlError::Migration`).

### Nontransactional migrations

Some statements refuse to run inside a transaction, such as PostgreSQL's
`CREATE INDEX CONCURRENTLY`. A migration declaring `Policy::NonTransactional`
runs its whole body without an artifact transaction:

```sql
-- Index titles without blocking writes.
-- rupa:
--   policy: nontransactional
CREATE INDEX CONCURRENTLY notes_title ON notes (title);
```

Its attempt is journaled before the body starts. Each statement of a script is
then sent on its own, because several statements in one PostgreSQL request share
an implicit transaction (and MySQL commits each DDL statement anyway). The script is split by the same grammar that validated
it as a whole before the first statement ran (`rupa_migrate::split_script`). After
the body returns, its attempt is recorded `applied`, still under the session lock. Transactional and
nontransactional migrations can be mixed in one catalog and run in version order.
Keep each nontransactional migration small: one artifact cannot mix policies.

Effects are not atomic. A body refused before it sent any statement, such as a
script the grammar rejects, records an ordinary failed attempt that a corrected
catalog may replace. Once a statement was sent, statements that already ran stay
committed. The attempt is therefore **not** recorded as failed, which would
permit a replay. It stays unresolved, and the run's `RunError::Migration` lists
`Error::Uncertain(attempt)` among its cleanup errors. The lock is released
normally, and every later apply fails with `Error::Uncertain` until an
operator [recovers](#recovery) the attempt. An interrupted body or a lost completion write leaves
the same state. Write nontransactional statements so an operator can inspect
what ran (e.g. `CREATE INDEX CONCURRENTLY IF NOT EXISTS`, then checking for an
invalid index).

### Interruption

A runner that dies mid-run (a crash, a killed process, a dropped future or a
lost connection) leaves its attempt `running`. There is no timeout takeover.

- **Transactional attempts reconcile themselves.** An attempt's completion
  commits with its body, so a `running` transactional attempt never committed
  once its session has ended. The next `apply` finds it under exclusion, records
  it `rolled_back`, and stops without running anything, failing with
  `Error::Reconciled(attempts)`. Apply again to run the artifact under a new
  attempt.
- **Nontransactional attempts need an operator.** Their statements may have
  run, so the next `apply` fails with `Error::Uncertain(attempt)` until the
  attempt is recovered.
- **SQLite keeps the dead runner's claim.** SQLite has no session locks, so the
  durable owner claim outlives its runner, and every later run fails with
  `Error::Busy`. Once you have established that the old runner is stopped,
  release the claim (below). Its transactional attempt then reconciles on the
  next `apply`.

### Recovery

Recovery records an operator's assertion about an interrupted run. It requires
a reason, accepts optional evidence, takes the same exclusion as `apply`, and
executes no migration body. The catalog checks the assertion:

Each assertion is one invocation on its own dedicated connection, which
`recover` closes, like `apply`. [`status`](#inspecting-a-target) lists the
attempt ids. Pick the one that matches what you verified:

```rust,ignore
use rupa::migrate::{Recovery, RecoveryAction, sync::Recover};

// The effects of uncertain attempt 7 are complete: record its artifact as
// installed with `Provenance::Recovered`.
let completed = Recovery::new(RecoveryAction::Completed(7), "index exists and is valid")
    .with_evidence("INC-1234");
catalog.recover(connect()?, &completed)?;
```

```rust,ignore
// Its effects were undone by hand: the next apply runs it again.
let restored = Recovery::new(RecoveryAction::Restored(7), "dropped the partial index");
catalog.recover(connect()?, &restored)?;
```

```rust,ignore
// SQLite: the runner holding the claim has stopped.
let release = Recovery::new(RecoveryAction::ReleaseClaim, "pod terminated");
catalog.recover(connect()?, &release)?;
```

The reason and evidence are stored on the attempt's journal row; a released
claim appends a `claim_released` row. `Completed` requires the catalog's
artifact to have the checksum the attempt ran. `Restored` does not, so a
corrected artifact may replace it. Recovery refuses with a typed
`RecoveryError` when there is no reason, the attempt is unknown or already
resolved, the catalog lacks or changed the artifact, or there is no claim to
release. It fails with `Error::Busy` while another runner owns the target. An
incorrect assertion is the operator's responsibility: check the database
effects, and that the old runner is stopped, before asserting anything. Never
edit the journal by hand.

Dynamic (registry) sessions, transactional work on MySQL,
nontransactional work on SQLite, snapshots/adoption,
authored down migrations and allowed late application are not yet
implemented. Their accepted
design is in [the migration contract](design/migrations-contract.md).
