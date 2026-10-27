# Migration system: discovery and proposed epic

Date: 2026-10-19. Status: discussion draft, not an approved implementation spec.
All new APIs and commands below are proposals; they do not exist yet.

## Recommendation

Build an optional, Upwell-independent migration engine around explicit,
immutable migration history. Support both SQL files and Rust migration structs
in the initial design, using the same history, planning, and execution contracts.
Portable schema operations are a separate scope decision from Rust authoring.

Keep three concerns distinct: describing changes, safely executing them, and
integrating them into application startup and operator commands. Entity-derived
schema comparison is a separate future initiative.

Both authoring formats are an owner requirement, confirmed 2026-10-19. Other
policies below remain proposals. This does not assign Rupa to a 1.0.0 release
merely because Upwell's planning model uses that release.

## What Upwell actually does

Inspected live GitHub issues, milestone membership, project fields, and tracking
PR metadata on 2026-10-19. Useful examples:

- [Conditional components epic #203](https://github.com/upwell-rs/upwell/issues/203).
- [Feature work item #209](https://github.com/upwell-rs/upwell/issues/209).
- [Leaf task #291](https://github.com/upwell-rs/upwell/issues/291).
- [Spec libraries epic #217](https://github.com/upwell-rs/upwell/issues/217).
- [Rate limiting epic #262](https://github.com/upwell-rs/upwell/issues/262).
- [Draft integration PR #212](https://github.com/upwell-rs/upwell/pull/212).
- [Release milestone](https://github.com/upwell-rs/upwell/milestone/1)
  and [Upwell Delivery project](https://github.com/orgs/upwell-rs/projects/1).

| Planning level | GitHub issue type | Project Role | Purpose |
|---|---|---|---|
| Top-level epic | Feature | Feature | User outcome, principles, example, boundaries, acceptance, child roadmap |
| Epic child | Feature | Work item | Coherent deliverable with its own contract and acceptance criteria |
| Leaf slice or spike | Task | Work Task | Small implementation or investigation with verifiable completion |
| Tracking PR | PR | Integration PR | Review the assembled epic against its integration target |

Actual sub-issue relations establish ownership; blocked-by relations establish
execution order. A checklist in the parent makes progress readable but does not
replace those relations. A leaf can also belong directly to the epic, as #317
does for its simplification gate. Not every work item needs task children on day one.

Project categorization is separate from issue type:

- **Epic:** the top-level epic, including on nested tasks and associated PRs.
- **Workstream:** the item's area, which can differ from its parent's area.
  Current choices include App & plugins, Tooling & generators, Maintenance,
  and several Upwell-specific areas.
- **Role:** the four values in the table above.
- **Status:** Backlog, Blocked, Up Next, In Progress, In Review, Done.
- **Milestone:** release membership, rather than hierarchy or implementation area.
- **Planning note:** contextual planning text.

Labels such as `enhancement` are used broadly; many tasks have no labels.
The Role and issue type carry the structural distinction. The live Epic option
list does not yet include every newer epic, so copying the fields also requires
populating their values consistently.

The recorded delivery convention is an epic branch with a draft tracking PR,
child PRs into that branch, then the epic into the release integration branch.
PR #212 currently demonstrates this: `feat/conditional-components/203` targets
`release/1.0.0`. Completed child issues are closed after their child PR lands,
while the epic stays open until its complete acceptance gate passes. GitHub's
default-branch closing behavior means intermediate merges need explicit issue
and checklist maintenance.

At the initial inspection, Rupa had no GitHub issues or milestones and only the default labels.
The owner chose a separate [Rupa Delivery project](https://github.com/orgs/upwell-rs/projects/2)
on 2026-10-19. It uses the same Role, Epic, Workstream, Status, and Planning note
field structure as Upwell. Rupa issues belong there, including Upwell integration
work authored in the Rupa repository. The release milestone remains undecided.

## Rupa baseline

The local checkout inspected is `bf2d798`; Upwell's sibling checkout is on
`feat/conditional-components/203`, so its source is integration-branch evidence,
not proof that every API is available in a published release.

Already available:

- Native sync and async executors, pools, transactions and savepoints.
- Optional dynamic driver selection, with native static dispatch as the default.
- PostgreSQL, MySQL, and SQLite drivers, including alternate drivers per engine.
- Raw SQL statements, typed parameters, and driver-level script facilities.
- Entity table and column metadata, including SQL types and nullability.

Missing or insufficient for migrations:

- No migration catalog, history store, planner, cross-process lock, or recovery protocol.
- The statement IR has CRUD, raw SQL, and security operations; no DDL model.
- Entity metadata is not a complete physical schema specification. It cannot
  explain historical renames or data transformations.
- Script execution is not a common migration contract. In particular, the sync
  MySQL driver's `script` currently splits on semicolons. It cannot be reused
  unchanged for arbitrary migration files containing literals or routines.
- The memory driver rejects raw SQL. It cannot establish SQL migration correctness.
- `rupa-upwell` has no dependencies and is still a placeholder. The historical
  prep document contains superseded API assumptions; current code takes precedence.

Relevant local sources: [architecture](../architecture.md),
[Upwell preparation](upwell-prep.md),
[statement IR](../../crates/rupa-core/src/ir.rs),
[entity contract](../../crates/rupa-core/src/entity.rs), and
[Upwell crate](../../crates/rupa-upwell/README.md).

## Approaches considered

| Approach | Strength | Cost / limitation |
|---|---|---|
| SQL-first runner, shared extension contracts | Smallest useful vertical; full database-specific SQL; independent of application models | Separate SQL per engine when needed; careful script handling |
| Rust-first with portable schema builders | Fits Rupa's Rust API and supports reusable schema operations | Requires a DDL model, renderers, capability checks, and historical type discipline up front |
| Entity snapshots and generated diffs | Convenient candidate migration generation | Largest scope; rename ambiguity, incomplete schema metadata, introspection, and data migration still need explicit handling |

The initial recommendation above was SQL-first. The owner subsequently required
both SQL and Rust files from the outset. Rust execution does not require a
portable schema builder: it can use the existing executor between Rust steps.
Entity diffing should generate reviewable artifacts if introduced; it should
never silently synchronize a production schema.

Existing ecosystem reference points are [SQLx's embedded migration runner](https://docs.rs/sqlx/latest/sqlx/macro.migrate.html),
[Refinery's SQL/Rust authoring](https://docs.rs/refinery/latest/refinery/), and
[SeaORM's schema builder migrations](https://www.sea-ql.org/SeaORM/docs/migration/writing-migration/).
These show viable authoring styles. Adopting one runner wholesale still needs an
adapter/dependency assessment against Rupa's native sync and async driver matrix;
SQLx-only execution would not cover that matrix by itself.

## First-release features and contracts

### Catalog and authoring

- Stable version IDs and descriptions, deterministic order, duplicate rejection.
  Timestamp-shaped numeric IDs are convenient; gaps are allowed.
- Immutable applied migrations. Reject changed checksums, missing history entries,
  and newly inserted versions below the applied frontier by default.
- Embedded sources for deployment; filesystem sources for tools. Embedding must
  rebuild when files are added, removed, or changed, not only when Rust changes.
- Explicit target dialect and engine/version requirements. A catalog can provide
  a complete engine-specific variant; missing variants fail before execution.
- Optional authored down migrations. Reversal is an explicit operation and may
  be destructive; it does not promise to restore deleted data.
- A manifest carries transaction requirements and execution restrictions.
  Avoid runtime templating of migration bodies.

Start with one ordered stream per migration target. Keep stream identity in the
history format so independently versioned plugin streams remain possible later.
Cross-stream dependencies and tenant fleet orchestration are later features.

### History, inspection, and planning

- Configurable, quoted history-table/schema identity; no interpolated identifiers.
- Versioned history format recording stream/version, source checksum and algorithm,
  selected dialect variant, description, timing, and execution outcome.
- Explicit attempt state for operations that can leave partial changes. Keep
  recovery history instead of rewriting away evidence of failed attempts.
- Read-only `status`, `validate`, and `plan`. Missing history means an empty
  history for inspection; inspecting must not create the table or schema.
- `validate` checks source/history consistency; `check` additionally requires
  the requested migration target to be satisfied. Neither proves physical schema
  equivalence after manual DDL changes.
- A plan lists exact steps, transaction guarantees, reversibility, and known
  requirements. Raw SQL is opaque: a plan cannot prove absence of destructive DDL.
- Apply revalidates history under the migration lock. A previously printed plan
  is advisory; stale plans cannot bypass fresh checks.

Checksums cover the executable artifact and relevant execution metadata using a
documented canonical encoding. Do not use Rust's unstable `DefaultHasher` as a
persistent format. SQL file bytes and engine variants have explicit identities.
Rust migration checksums require a separate artifact/dependency strategy; hashing
only a function's tokens misses changes in helpers.

### Execution and recovery

- Hold one dedicated connection for the complete migration session. Lock,
  history reads, execution, and completion must refer to the same target.
- Use database-coordinated locking across processes, including first-run history
  creation. PostgreSQL/MySQL session locks and SQLite writer serialization are
  candidates to prove, not interchangeable implementations of one atomicity claim.
- Locks need a deterministic target identity shared across drivers and bounded
  waiting. Start with database-wide serialization so streams cannot race over
  shared objects. SQLite's ability to hold exclusion across transaction boundaries
  needs a dedicated spike; reject unsupported combinations rather than run unlocked.
- Transaction policy is a typed requirement: `Required` or explicitly
  `NonTransactional`. Never silently downgrade `Required` to best effort.
- For supported transactional work, schema changes and the applied-history record
  commit together, one migration at a time. Earlier successful migrations remain
  applied if a later migration fails.
- For nontransactional work, persist an attempt before execution. A crash between
  DDL and bookkeeping leaves an uncertain outcome. Block automatic replay and
  require inspection plus explicit recovery; do not claim exactly-once execution.
- Errors identify migration, phase, statement/source location when available,
  underlying driver cause, and observed recovery state. Preserve cancellation
  safety and clean or discard connections before reuse, including session locks.
- Never retry arbitrary DDL or Rust migration bodies automatically.
- Define statement/script grammar per backend. Support semicolons in strings,
  comments and PostgreSQL dollar quotes. Document unsupported client syntax
  such as `DELIMITER`, psql commands, and SQLite shell directives.

Database limitations drive these requirements: [MySQL atomic DDL](https://dev.mysql.com/doc/refman/8.0/en/atomic-ddl.html)
does not make a multi-statement migration transactionally atomic;
[PostgreSQL concurrent index creation](https://www.postgresql.org/docs/current/sql-createindex.html)
cannot run in a transaction block. Portable alter-table work must also account
for [SQLite's documented alteration/rebuild procedures](https://www.sqlite.org/lang_altertable.html)
and Rupa's supported SQLite baseline rather than assuming the latest engine.

### Operator experience

- Create migration files; show status; validate; plan; apply through a version;
  explicitly revert through a version; machine-readable reports and exit codes.
- Explicit baseline/adoption of an existing database, recording what was assumed.
  A baseline command cannot prove an arbitrary live schema matches the sources.
- Explicit recovery for failed/uncertain attempts; no blanket checksum-rewrite
  or ignore-history flag as the default escape hatch.
- Separate migration credentials from runtime repository credentials.
- Per-target execution with no implied transaction across multiple databases.
- Real SQL database tests for each driver, plus shared planner/history tests.

## Proposed Rust API

The API should distinguish pure source validation from database inspection and
execution. Names below illustrate the shape; signatures need review.

```rust
use rupa::migrate::{embed_migrations, Migrator, Target};

let migrations = embed_migrations!("migrations");
migrations.validate()?;

let migrator = Migrator::new(&migrations);
let mut conn = pool.get()?;
let plan = migrator.plan(&mut conn, Target::Latest)?;
let report = migrator.apply(&mut conn, Target::Latest)?;
```

Connection-based methods make ownership and session-lock lifetime explicit.
Pool convenience methods can acquire a connection and delegate.

Native async uses the same operation names through an `AsyncMigrator`:

```rust
let migrator = AsyncMigrator::new(&migrations);
let mut conn = pool.get().await?;
let report = migrator.apply(&mut conn, Target::Latest).await?;
```

`status`, `validate`, `plan`, `apply`, `revert`, `baseline`, and `recover` are
distinct operations. Inspecting a target below the current version must not
implicitly select reversal. Use typed options and structured reports/errors.

Suggested crate boundary: `rupa-migrate` owns catalog, history model, planner,
runner, and migration-specific execution contracts. Driver adapters implement
the needed session/script/lock operations without depending on Upwell. Resolve
trait ownership versus driver dependency cycles before adding crates; a small
shared contract belongs in core only where required. Re-export from the facade
through `migrate`; `async` and `dynamic` remain separate opt-ins.

Rust bodies should receive an exclusive migration context exposing
the session executor and explicit schema operations. Prefer native generic
traits with unboxed async futures; an enum or generated catalog can dispatch
heterogeneous migrations. Add boxing only for the dynamic path. Historical
migrations should use frozen migration-local types rather than current app
repositories/entities, which may no longer match the old schema.

Future schema syntax could look like this:

```rust
ctx.schema().create_table("users")
    .column("id", Type::BigInt, ColumnOptions::primary_key())
    .column("email", Type::Text, ColumnOptions::required())
    .execute()?;
```

Treat this as an ergonomics example only. Portable types, generated IDs,
constraints, indexes, defaults, schema namespaces, and SQLite rebuild semantics
need their own design before committing to a builder surface.

## Upwell integration

Put the adapter in `rupa-upwell`, consuming the standalone engine. Typed pools
remain usable directly; configured dynamic `Database` is an optional route.
Applications explicitly register migration sets against qualified database
targets. Linking an entity or repository must not silently register schema work.

Proposed user-facing configuration:

```toml
[database.migrations]
startup = "check"             # off | check | apply
lock_timeout = "30s"
```

This is proposed adapter configuration, not an existing `DbConfig` option.
Recommend `check` for applications opting into the adapter; `apply` is an
explicit startup policy, useful locally and for suitable deployment models.
Operators can instead run a one-shot migration job with elevated credentials.

Startup ordering must be explicit:

1. Resolve configuration and migration-target metadata.
2. Open the selected migration connection with the selected credentials.
3. Check or apply migrations according to policy.
4. Construct schema-dependent application components.
5. Start listeners, jobs, and other request-producing work.

Upwell already has `Setup`, `PreBuild`, and `Built` CLI phases and a
`DatabaseMigrateCommand` example using `Built`. That example resolves a database
after construction; it is not a reusable Rupa migration implementation. An
`after_build` migration can be too late if component initialization queries the
new schema. First implement a command at Setup/PreBuild that obtains its
connection without constructing the ordinary application graph; verify the
required config access before choosing the final phase.

Proposed command surface:

```text
my-app database migrate status --database primary
my-app database migrate validate --database primary
my-app database migrate plan --database primary --to latest --format json
my-app database migrate apply --database primary --to latest
my-app database migrate revert --database primary --to 20261018090000
```

A manually authored typed `CliCommand` can establish the integration before
plugin-contributed commands are available. Contributing the command group via
Upwell's plugin CLI is additive. `cargo upwell inspect` may expose static migration
metadata through the tooling extension seam, but must not connect or run SQL.

Startup failures prevent normal service activation. Readiness checks use declared
schema requirements, not simply the maximum applied version. During rolling
deployments, a newer schema may be compatible with an older binary; migration
execution validation and runtime schema compatibility are separate policies.
Design an explicit compatibility contract and expand/backfill/contract runbook
before claiming rolling-upgrade support.

Do not execute DDL during speculative conditional-component construction or
config reload preparation. Upwell can abort a candidate graph; that cannot undo
committed database changes. A database-target reload must check compatibility
or require a separately managed migration, with no automatic down migration.

## Proposed GitHub epic and work items

Top-level title: **Epic: explicit database migrations and Upwell integration**.
Type Feature, Role Feature. Give every descendant and associated PR the same
top-level Epic field; select Workstream independently. Choose a Rupa milestone
after deciding the release boundary. Codes below are draft identifiers, not
GitHub issue numbers.

| ID | Feature work item | Initial Task slices | Depends on |
|---|---|---|---|
| M1 | Semantics and execution guarantees | Version/order rules; failure-state model; backend locking and transaction spikes | — |
| M2 | Migration catalog and source artifacts | SQL/Rust grammar; generated SQL structs and Rust module registration; checksums and variants; embedding/rebuild tracking | M1 |
| M3 | History and deterministic planning | History format/bootstrap; read-only status/validation; targets and stale-plan checks | M1, M2 |
| M4 | Driver execution and lock contracts | Native sync/async session contract; PostgreSQL; SQLite; MySQL script and lock support; dynamic adapter | M1 |
| M5 | Apply, revert, and recovery runner | Declarative and imperative traits; sync/async adaptation; transactional vertical; uncertain outcomes/cancellation; authored reversal; baseline/recovery | M2, M3, M4 |
| M6 | Operator commands and reports | Command contract; human/JSON output and exit codes; adoption/recovery runbook | M5 |
| M7 | Upwell registration, startup, and CLI | Qualified targets/credentials; pre-construction schema gate; typed command; plugin/tooling contribution | M5; M6 for shared command behavior |
| M8 | Conformance and epic acceptance | Concurrent first startup; crash/failure matrix; driver interoperability; feature/dependency matrix; simplification gate | All required deliverables |

Begin acceptance tests alongside M3–M5, rather than deferring verification until
M8. First vertical: embedded SQLite migration, history, lock, status, and apply;
then PostgreSQL and MySQL native drivers, followed by alternate-driver parity.
M1 must settle unsupported transaction/lock combinations before their adapters
are described as complete.

Candidate later epics:

- **Portable schema operations:** DDL model/rendering and capability validation,
  usable from the initial Rust migration authoring path.
- **Schema inspection and candidate generation:** introspection, snapshots,
  rename hints, drift reports, reviewable generated changes.
- **Large deployments:** resumable backfills, independent plugin streams,
  dependency ordering, tenant fleets, and compatibility orchestration.

Each work-item issue should contain: parent/epic reference, concrete goal,
baseline, scope, guarantees and failure behavior, dependencies, acceptance
criteria, and explicit exclusions. An investigation task needs a decision and
evidence as its acceptance result, not an implied implementation commitment.

The epic closes only when a fresh database and an existing database can be
migrated predictably through every supported SQL driver; concurrent runners
cannot duplicate work; changed history is rejected; interruption produces a
documented recovery state; and an Upwell command can migrate without starting
services or constructing schema-dependent components first.

## Decisions to settle before filing implementation issues

1. Declarative versus imperative trait contracts and their sync/async adapters.
2. First release: all SQL engines, or a deliberately smaller supported subset.
3. Whether authored reversal and baseline/recovery ship in the first release.
4. Target Rupa milestone and shared versus separate delivery board.
5. Baseline kinds, out-of-order policy, version grammar, and history/rollback semantics.
6. Whether portable schema builders join this initiative.

Both SQL and Rust authoring are required. The decisions above need discussion
before the issue tree is treated as an implementation roadmap.

## Discussion expansion: authoring and migration policies

The following is a decision inventory and proposed direction, not settled API.

### Declarative and imperative migrations

Use three conceptual contracts: shared metadata, a pure migration plan, and an
executor-driven migration. Suggested names are `MigrationInfo`,
`PlannedMigration`, and `Migration` / `AsyncMigration`.

- A SQL file generates a zero-sized struct with static metadata and embedded
  source. It implements `PlannedMigration`; library adapters provide sync and,
  when enabled, async execution.
- A Rust module exports a migration struct. It can implement `PlannedMigration`
  if it only describes operations, or `Migration` / `AsyncMigration` when it
  needs to inspect query results and execute Rust between database steps.
- A declarative plan contains ordered SQL/statement operations and execution
  requirements. Queries with different result types cannot simply be collected
  into a homogeneous `Vec<Query<R>>`; use a migration-specific operation model.
- A Rust body takes a restricted context bound to the session selected by the
  runner. It can run typed queries but cannot silently commit the runner's
  transaction, release its lock, or acquire a different session.
- Generated SQL wrappers inherit the library's supported execution modes. A
  genuinely async Rust migration does not acquire a sync implementation just
  because both facade features are enabled. It must provide one explicitly,
  or be excluded/rejected for a sync catalog. Never hide block-on behavior.
- Plan-to-execution blanket implementations reserve those impls: a type cannot
  also override imperative execution. Alternatively use `Planned<T>` adapters
  to preserve that flexibility. This coherence choice is still open.
- Macro feature selection must follow the facade's forwarded capabilities;
  emitted `cfg(feature = "async")` in a consuming crate tests the wrong crate's
  features. Do not require applications to mirror internal feature names.
- Rust files are compiled modules, not runtime scripts. Decide whether each
  exports a conventional `Migration` struct or declares its exported type in a
  manifest. The struct need not be zero-sized if construction is explicit.

The pure form gives full operation visibility without running user migration
code. The imperative form can expose metadata and declared phases, but a dry
run cannot safely discover queries by executing its body against production.
Shared engine code can interpret pure plans for both sync and async execution.

Distinguish three meanings of splitting: SQL lexical statement boundaries,
explicit transaction phases, and resumable progress checkpoints. Pure plans can
declare the latter two, but SQL splitting alone proves neither. A Rust operation
between queries is not automatically a safe commit/retry boundary. External IO
cannot be rolled back with the database; its replay contract must be explicit.

### Baseline has two distinct meanings

1. **Adoption marker:** record that an existing unmanaged database is considered
   equivalent to history through version N. Execute no historical migration SQL.
2. **Executable baseline/snapshot:** initialize a fresh target with one artifact
   covering the effects of migrations through N, then run later migrations.

An ordinary incremental migration changes the previous state; a snapshot creates
the state at its coverage boundary. These are independent of SQL versus Rust
authoring. Snapshot and incremental artifacts may share a version but have
different identities/kinds. An adoption marker is a history operation, not an
executable migration.

Recommend explicit adoption only. Snapshot selection requires a fresh owned
schema/target, not merely absence of a history table. A nonempty unmanaged target
must not automatically receive a create-everything baseline. Managed targets
continue their incremental path and never run a newer snapshot on top.

History distinguishes applied, covered by snapshot, and adopted states. Covered
does not mean individually executed. Choose the newest compatible snapshot at
or below the requested target, or replay the full incremental chain if available.
Reverting below a snapshot/adoption boundary is rejected unless an explicit
supported reverse path exists; old incremental down scripts cannot be assumed
to reverse a snapshot-created database.

Keep old sources initially. Pruning needs a compatibility/retention policy for
databases still on the incremental path. Snapshot coverage must identify which
artifacts it includes: introducing an older migration later must not silently
declare it covered just because its version is below N. CI should compare full
replay with snapshot-plus-tail, including required data effects.

Reference: Flyway also distinguishes its [baseline command](https://documentation.red-gate.com/flyway/reference/commands/baseline)
from [executable baseline migrations](https://documentation.red-gate.com/flyway/flyway-concepts/migrations/baseline-migrations).
Rupa's exact semantics remain a design choice.

### Out-of-order execution

Example: a database applied versions 100 and 102; version 101 arrives later.
Strict mode rejects the late artifact; an explicit allow policy applies 101
after 102. It does not reorder already executed history. Recommend strict by
default, with out-of-order execution supported as an explicit policy.

Persist both logical version and actual installation rank. Plan/report output
must identify late execution. A timestamp does not prevent this situation:
branch merge order can differ from migration creation order.

The author owns compatibility with both 100→101→102 and 100→102→101 paths.
CI can verify supported paths but the planner cannot prove SQL commutativity.
Decide whether permission is per run, per migration, or both. Avoid silently
skipping a late version. A maximum version alone is insufficient to represent
pending work or runtime schema requirements.

Rollback needs a deliberate contract. Actual installation order is the natural
stack for undo; a logical target may require undoing later effects and replaying
some migrations. Prefer rejecting ambiguous target reversal initially and
providing an explicit plan rather than sorting down scripts by version and
assuming that reverses the database. Snapshot coverage and adoption boundaries
remain authoritative even when out-of-order execution is allowed.

### Numbered versus timestamp versions

Both need numeric/structured comparison, unique identities, and deterministic
ordering. Never lexically order unpadded numbers (`10` must sort after `2`).

- Sequential versions are readable and easy to target, but branches compete
  for the next number. Unapplied collisions can be renumbered; applied artifacts
  must retain their identities.
- UTC timestamp versions reduce coordination but do not eliminate same-instant
  collisions, clock skew, or out-of-order merges. Define precision and collision
  handling, and never silently replace an artifact.
- Recommend one version scheme per stream, selected explicitly. Support both;
  use UTC timestamp IDs as the generator default. Treat time-shaped IDs as an
  ordering convention, not proof of dependency order.

Choose whether dotted versions are needed before defining the parser. A plain
numeric scheme plus a timestamp scheme is smaller than arbitrary semver-like
ordering. Also decide whether version itself is identity or whether artifacts
carry a separate immutable ID; this affects renames, coverage manifests, and
cross-stream dependencies.

### Additional policy questions to inventory

- Repeatable migrations and their order relative to versioned migrations.
- Explicit dependency edges versus a single linear stream.
- Down migrations, roll-forward-only migrations, and irreversible boundaries.
- Exact source hashing, Rust helper dependencies, and migration construction.
- Missing historical sources, permitted pruning, and branch switching locally.
- Engine variants, schema ownership, search paths, and multiple databases.
- Transaction phases, checkpoints, cancellation, and external side effects.
- Runtime compatibility during rolling deployments, including late migrations.
- Seeding/reference data versus long-running resumable backfills.

This inventory should become a semantics/discovery work item before detailed
implementation tasks are finalized. The initial eight-item outline describes
areas of work; it is not evidence that these policies have already been settled.

## Drafted GitHub issue tree

The discussion is tracked in [epic #3](https://github.com/upwell-rs/rupa/issues/3).
Project: [Rupa Delivery](https://github.com/orgs/upwell-rs/projects/2), separate
from Upwell Delivery.
Issue bodies are explicitly discussion drafts; no release milestone has been
chosen. Both SQL and Rust authoring are required. The trait-adapter decision
remains open in [task #15](https://github.com/upwell-rs/rupa/issues/15).

| Area | Feature work item |
|---|---|
| Semantics and policy decisions | [#4](https://github.com/upwell-rs/rupa/issues/4) |
| SQL/Rust authoring and catalog | [#5](https://github.com/upwell-rs/rupa/issues/5) |
| History and planning | [#6](https://github.com/upwell-rs/rupa/issues/6) |
| Driver sessions and locking | [#7](https://github.com/upwell-rs/rupa/issues/7) |
| Execution, reversal and recovery | [#8](https://github.com/upwell-rs/rupa/issues/8) |
| Operator commands and runbooks | [#9](https://github.com/upwell-rs/rupa/issues/9) |
| Upwell integration | [#10](https://github.com/upwell-rs/rupa/issues/10) |
| Conformance and acceptance | [#11](https://github.com/upwell-rs/rupa/issues/11) |

Discovery tasks under #4 cover [versions/identity #12](https://github.com/upwell-rs/rupa/issues/12),
[baselines #13](https://github.com/upwell-rs/rupa/issues/13),
[out-of-order execution #14](https://github.com/upwell-rs/rupa/issues/14),
[trait adapters #15](https://github.com/upwell-rs/rupa/issues/15),
[transactions/locks #16](https://github.com/upwell-rs/rupa/issues/16),
[artifacts/checksums #17](https://github.com/upwell-rs/rupa/issues/17), and
[scope/repeatables #18](https://github.com/upwell-rs/rupa/issues/18).
