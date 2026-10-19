# Migration contract and implementation scope

Status: consolidated proposal for owner review. Completes the remaining
semantics discovery in [#4](https://github.com/upwell-rs/rupa/issues/4):
out-of-order/reversal #14, authoring #15, execution guarantees #16, artifacts
#17, and initial scope #18. No APIs described here are implemented yet.

The accepted [version/identity](migrations-version-identity.md) and
[adoption/snapshot](migrations-baselines-adoption.md) contracts remain in force.
This document resolves their remaining handoffs together. Approval of this
proposal completes discovery; subsequent PRs deliver working migration paths,
with tests and documentation alongside the code.

## Initial scope and implementation order (#18)

Support one explicitly owned SQL target and one linear versioned stream per
invocation. Support both embedded SQL and compiled Rust migrations from the
first working vertical. Keep native static sync execution available without
async or dynamic features; async futures are unboxed by default. Optional
runtime selection adapts to the same engine. Never run automatic production
schema synchronization.

| Capability | Initial scope and rationale |
|---|---|
| SQL and Rust authoring | Included; one catalog, identity, and history contract |
| Numbered and UTC versions | Included as accepted in #12 |
| Planned and imperative bodies | Included with explicit adapters below |
| PostgreSQL, SQLite, MySQL | Included through the precise policy matrix below; do not claim uniform DDL atomicity |
| Authored down migrations | Included; irreversible and ambiguous paths fail before execution |
| Adoption and snapshots | Included as accepted in #13, with immutable coverage |
| Out-of-order execution | Included by explicit per-run permission; strict by default |
| Attempts and operator recovery | Included; no blind replay of uncertain effects |
| Standalone library and reusable commands | Included; no requirement to add a cargo-rupa binary |
| Upwell startup gate and application commands | Included after the standalone vertical; configuration/phase prerequisites must be verified in #10 |
| Repeatables | Deferred; checksum-triggered reruns need separate ordering and undo semantics; initial catalogs reject them |
| Dependency DAG and multiple streams per invocation | Deferred; linear order is sufficient initially; never infer cross-stream dependencies |
| Reference data | Included as ordinary versioned SQL/Rust work |
| Long-running resumable backfills and external-effect orchestration | Deferred; neither lexical statements nor Rust steps prove safe checkpoints |
| Portable schema operations | Included (amended in #40): dialect-free DDL in plans, rendered per session dialect, unsupported forms rejected before acquisition |
| Entity diff generation and introspection | Deferred; generated changes must be reviewable artifacts, never automatic synchronization |
| Pruning and automatic schema equivalence | Deferred as accepted in #13; retain historical sources |
| Tenant fleets and automatic down on reload | Excluded; target selection and reversal remain explicit |
| Release milestone | Unassigned; do not infer a Rupa release from Upwell's version |

Deliver useful verticals rather than more discovery PRs:

1. Mixed SQL/Rust catalog, validation, history, and transactional SQLite apply
   on a dedicated native sync connection, including rollback and concurrency
   tests. This is the first implementation PR, covering bounded portions of
   #5/#6/#7/#8 with a runnable example.
2. Async execution and PostgreSQL sessions; shared fixtures prove parity and
   native driver interoperability, including nontransactional attempts.
3. MySQL nontransactional DDL and alternate-driver adapters; complete the
   supported engine matrix and interruption/recovery behavior.
4. Complete snapshot/adoption/reversal/operator flows, then Upwell integration;
   shared conformance grows in each PR rather than waiting until #11.

These are implementation groupings, not permission to omit acceptance tests
or leave the epic done after the first vertical. The exact PR boundary may
follow code size and reviewability; no further one-topic discovery PRs are
needed for #14–#18.

## Authoring and crate boundaries (#15)

Choose explicit `Planned<T>` execution adapters. Shared metadata, static plans,
and imperative execution are separate contracts. A plan contains ordered SQL
operations with no returned query values; Rust code needing query results uses
an imperative body. SQL and planned Rust can share one interpreter without
reserving every author's execution impl through a blanket implementation.

Proposed ownership:

- `rupa-migrate`: catalog, metadata, operations, contexts, session contracts,
  history, planner, runner, reports, and explicit plan adapters. Depends on
  `rupa-core`, never on concrete drivers or Upwell.
- Drivers: optional implementations of migration session/script/lock contracts;
  their migration feature depends on `rupa-migrate`, preventing a dependency
  cycle. SQL lexical handling shared by drivers belongs in `rupa-migrate`.
- `rupa-macros`: SQL ZST/catalog generation, with paths through the facade's
  reexports and no runtime dependency on concrete drivers.
- `rupa`: optional `migrations` feature, reexporting the engine and forwarding
  migration support only to enabled drivers. `async` and `dynamic` retain
  their existing independent meanings.
- `rupa-upwell`: consumes the standalone engine; never owns a parallel runner.

Use these conceptual contracts; final signatures must be validated by the
first implementation's compile tests:

| Contract | Responsibility |
|---|---|
| `MigrationInfo` | Static identity, description, execution policy, checksum, capabilities and source diagnostics |
| `PlannedMigration` | A pure `plan()` of SQL and schema operations, evaluated once at catalog construction; that plan is checksummed and executed (amended in #40: entity-derived tables cannot be static arrays) |
| `Migration<S>` | Sync body over an exclusive restricted sync context for session `S` |
| `AsyncMigration<S>` | Async body over an exclusive restricted async context, returning a `Send` future |
| `Planned<T>` | Implements the applicable sync/async execution contracts by executing the plan its catalog entry recorded |

Illustrative authoring forms below are design examples, not compiling public
API signatures. All three carry the same identity/checksum metadata.

```text
SQL file 20261019093000_create_notes.sql:
    CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL);
Generated: CreateNotes (ZST), MigrationInfo + PlannedMigration
Catalog execution entry: Planned(CreateNotes)

Planned Rust module:
    pub struct AddIndex;
    impl PlannedMigration for AddIndex:
        UP = [Sql("CREATE INDEX notes_title ON notes(title)")]
        DOWN = [Sql("DROP INDEX notes_title")]
Catalog execution entry: Planned(AddIndex)

Imperative Rust module:
    pub struct NormalizeTitles;
    impl Migration<S> for NormalizeTitles:
        rows = context.run(typed_select_notes)?
        for row in rows:
            title = normalize_in_rust(row.title)
            context.run(typed_update_title(row.id, title))?
Catalog execution entry: NormalizeTitles
Async variant explicitly implements AsyncMigration<S> and awaits context.run.
```

`Planned<T>` has one execution error type wrapping a driver/session error and
operation diagnostics. An imperative type chooses a typed error convertible
from the session's error; catalog code converts each branch to a generated
error enum preserving sources and artifact identity. Default static execution
must not require `Box<dyn Error>` or boxed futures. A declared dynamic catalog
may erase errors/executors explicitly.

Generate one closed catalog enum over the declared concrete entries, forwarding
metadata and sync/async execution by match. A sync catalog requires every entry
to support sync; an async catalog requires every entry to support async.
Generate only selected compatible modes; a genuinely async-only Rust body
never acquires sync execution or hidden block-on. Store concrete values in the
enum so explicit Rust construction can carry state. Captured values affecting
behavior must be immutable, declared checksum inputs; do not capture secrets
or runtime-varying behavior in a migration artifact.

SQL wrappers implement the static plan contract; the generated catalog applies
`Planned` automatically for SQL entries. Rust catalog entries explicitly select
planned adaptation or their imperative implementation. Macro capability flags
come from facade-to-macro feature forwarding, never emitted consumer-crate
`cfg(feature = "async")`. Compile fixtures must cover consumers without
matching local feature names, sync-only, async-only, dual-mode and dynamic use.

A context exposes typed `run` and migration script execution on the selected
session. It does not expose its connection, pool, transaction, lock guard,
commit/rollback, or transaction acquisition. Runner control traits are not
implemented on it. Validate script/raw-query entry points against the supported
SQL grammar and reject transaction/session control and reserved metadata access.
No raw connection escape hatch belongs in the migration context.

This is an ownership boundary, not a sandbox: trusted Rust can independently
open connections or perform external IO, and SQL can invoke user-defined
functions with effects. Such behavior violates the migration contract if it
changes runner control or performs undeclared external work. Never claim Rust
or SQL validation proves arbitrary program purity. Test ordinary accidental
control attempts through the provided API and document the trust boundary.

`Planned<T>` avoids the coherence restriction of `impl<T: PlannedMigration>
Migration for T`: authors may implement custom execution on `T` and select the
adapter separately. SQL-specific generated execution impls were rejected as
extra duplicated interpretation; metadata alone never implies execution mode.

Inspection reads static metadata/operations and history. It never executes an
imperative body, constructs arbitrary user factories to discover operations,
or substitutes a production dry-run executor. A planned dry run shows exact
static SQL; an imperative dry run shows identity, policy, source and opacity.

## Ordering, history, and reversal (#14)

Strict mode validates the whole requested plan before writes. A missing
incremental with version below the greatest active executed incremental is a
late artifact, and blocks apply unless the invocation explicitly allows late
work. Exclude already covered/adopted members from pending work; an uncovered
artifact at or below their boundary always blocks, regardless of this option.

Choose per-run permission, not another per-artifact flag: it makes the operator
acknowledge the actual target's order. The author remains responsible for both
supported orders; the engine cannot prove SQL commutativity. A late permission
never suppresses missing-source, checksum, uncertain-attempt or scheme errors.

Append a monotonically increasing installation rank on successful forward
execution, separate from logical version. Reversal appends its own event and
marks the prior installation inactive; never delete/reuse rank or artifact
history. Reapplication of the same immutable artifact gets a new installation
rank. Covered/adopted members have provenance rather than fictitious individual
installation ranks. Maintain the greatest active executed version for late-work
checks; audit history still retains versions undone earlier.

Undo follows reverse installation order of active executed incrementals. Offer
an explicit last-installations count for cases without a meaningful version
target. A logical target may undo only a suffix of that stack: after selecting
active entries above the target, reject if a retained entry is interleaved in
that suffix. Never undo and silently replay retained migrations. Preflight all
down availability, policy support and coverage boundaries before the first undo.
Each successful down is committed separately; a later failure preserves earlier
completed undos and reports the remaining active stack.

| History / request | Result |
|---|---|
| `100`; apply catalog `100, 101, 102` | Execute `101`, then `102` |
| `100, 102`; catalog adds `101`, strict apply | Reject before running `101` |
| `100, 102`; explicit late permission | Execute `101` after `102`, with a new rank; report late work |
| Active install order `100, 102, 101`; revert to `100` | Undo `101`, then `102`, if both downs exist |
| Active install order `100, 102, 101`; revert to `101` | Reject: undoing only `102` is not a suffix |
| Same order; undo last one | Undo `101`; retain active `100, 102` |
| Then strict apply with catalog including `101` | Reject late `101`; reapply needs explicit permission |
| Covered/adopted through `102`; unlisted `101` appears | Reject even with late permission |
| Requested suffix contains an irreversible migration | Reject entire reversal plan before executing any down |

Runtime compatibility must specify required immutable artifact identities and
acceptable provenance, not just a minimum numeric version. A late artifact can
be missing above or below a database's maximum version. Startup check fails if
required identities are absent/mismatched or recovery is uncertain; never infer
compatibility from install rank alone. Rolling deployments require the author
to design compatible transitions; no automatic down occurs on config reload.

## Dedicated sessions, policies, and exclusion (#16)

One runner owns one dedicated physical connection for lock acquisition,
bootstrap, history, execution and cleanup. A pool may supply it, but ordinary
checkout must not release/reacquire it between operations. Reject a connection
already inside an application transaction. Never expose a migration connection
to repositories outside its context during a run.

Two execution policies initially: `TransactionalRequired` and `NonTransactional`.
There is no automatic downgrade and no mixed transaction phases inside one
artifact. SQL metadata declares its policy; Rust metadata covers the whole body.
Separate artifacts must model steps requiring different policies. Lexical SQL
boundaries are not transaction boundaries or resumable checkpoints.

| Engine / drivers | TransactionalRequired | NonTransactional | Exclusion across per-artifact commits |
|---|---|---|---|
| SQLite: rusqlite sync, sqlx async | Supported for allowed transactional SQL/DML; one transaction per artifact | Rejected initially, including VACUUM and migration PRAGMAs that require leaving the transaction | Durable cooperative owner claim described below |
| PostgreSQL: postgres sync, tokio-postgres async, sqlx async | Supported for server-allowed transactional SQL | Explicit policy supports operations such as concurrent index creation, with uncertain-effect recovery | Session advisory lock held until explicit release/session close |
| MySQL: mysql sync, sqlx async | Rejected initially for all migration bodies; no partial DML-only exception | Supported with InnoDB history tables and attempt journal; DDL may implicitly commit | Named session lock held across commits |
| Memory driver | Rejected for SQL migration conformance | Rejected | Use fake sessions for planner tests, not SQL correctness |

The selected adapters remain gated until their conformance fixtures pass.
Bundled/unbundled SQLite configurations follow the same contract. Dynamic
adapters use those sessions, never different lock/history conventions. SQLite
in-memory targets are session-local unless configured explicitly otherwise;
interprocess lock assertions require a shared file target.

PostgreSQL uses a single session advisory lock derived from canonical target
identity before history creation. MySQL uses one named session lock likewise;
no attempt may acquire it through a different connection or autocommit pool
operation. Use try-acquire/bounded waiting and verify every lock result. No
nested acquisition or reliance on connection-pool resets to release locks.

Define target lock identity independently of driver name, credentials, URL
spelling and stream version. Versioned canonical bytes consist of a domain tag,
engine, selected database and owned namespace, UTF-8 length-prefixed components.
Use SHA-256 of these bytes: PostgreSQL takes the first eight bytes as a signed
big-endian advisory key; MySQL uses a `rupa:` prefix plus 56 lowercase hex digits
(61 characters). Collisions can over-serialize, never bypass exclusion. Record
canonical target metadata in history and reject mismatched ownership. All
streams sharing an owned namespace use the same exclusion identity.

For SQLite, `BEGIN IMMEDIATE` excludes competing writers only until commit; it
cannot by itself guard a whole run with separately committed migrations.
Propose a durable owner-claim row in reserved metadata, acquired atomically
under `BEGIN IMMEDIATE` before user work. Bootstrap and claiming are one
transaction. Other runners wait or fail while an owner is present, even between
artifact transactions. The owner verifies its token before each write phase and
clears it transactionally on clean completion. No lease expiry or timeout may
steal it; a crashed/cancelled runner can leave a stale claim requiring explicit
recovery after the operator proves the old runner is stopped. Recovery replaces
the claim atomically under the same writer exclusion and records that assertion.
This conservative protocol trades unattended restart for exclusion without a
new OS-lock dependency or an unsafe lease. Cross-driver tests must verify it.

Reserved metadata names and schema signature are framework constants. To break
SQLite's first-run bootstrap cycle, inspect the owned namespace and classify
existing objects under the initial writer transaction before creating reserved
metadata. Create only on a fresh target or an explicit adoption operation;
refuse incompatible same-named objects. Signed, structurally valid internal
bootstrap objects do not themselves make a user target nonempty. They also do
not count as applied migrations. Other engines acquire session exclusion first,
then classify and create history using the corresponding supported protocol.

Lock, statement and total-run deadlines are explicit. Lock timeout means no
user body ran; statement/run cancellation stops dispatching new work, attempts
rollback/cleanup and can leave uncertainty. A server that has not acknowledged
cancellation may still be executing: do not grant recovery solely because a
client-side timeout expired. A lost connection ends this runner; reconnect only
for explicit reconciliation under newly acquired exclusion, never to continue
the same body. These locks exclude cooperative runners, not unrelated clients;
operator ownership must exclude external DDL.

## Attempts, interruption, and recovery (#16)

Persist an attempt with artifact identity, checksum, direction, policy and
expected history state before dispatching user work. No body is automatically
retried. Transactional completion updates history, the attempt and any snapshot
coverage in the same transaction as the artifact's database changes.
Nontransactional completion cannot be atomic with those effects; keep its
outstanding attempt durable until completion is confirmed.

| Interruption window | Known state and required next action |
|---|---|
| Before exclusion or successful bootstrap/attempt persistence | No user work dispatched; acquire/revalidate before a new invocation |
| Transactional body fails, rollback acknowledged | No effects from this attempt; record failure; a new explicit apply may start a new attempt |
| Transactional connection lost before/while committing | Client outcome unknown; after old session ends, reacquire exclusion and reconcile atomically recorded history/attempt; never infer failure from missing response |
| Transactional reconciliation finds committed completion | Preserve success and report it; do not replay |
| Transactional reconciliation finds no completion after rollback is established | Mark failed/rolled back; only a new explicit invocation may execute again |
| Nontransactional attempt persisted, no known dispatch | Conservatively uncertain after interruption; cannot prove the next instruction never ran |
| Nontransactional partial body or completion-write loss | Effects may exist; block apply/revert/adoption until explicit recovery |
| Completion durable, cleanup/release fails | Keep completed history; report cleanup failure and discard session; never rerun the completed artifact |
| Async future dropped without awaited cleanup | Discard/close dedicated connection; SQLite claim may persist; retain outstanding attempt for reconciliation |

Recovery is a named operator operation with an audit reason and evidence. For
uncertain nontransactional work, allow an explicit assertion that effects are
complete (record reconciled completion) or were manually restored to pre-state
(record abandonment before a new explicit apply). Neither path executes a body
as a probe, deletes prior attempts, or silently treats a failed artifact as
applied. Record recovered provenance distinct from normal completion. An
incorrect assertion is operator responsibility; reject unless exclusion and
original-runner termination are established.

On normal failure, attempt rollback before releasing the lock. A connection
with uncertain transaction or lock cleanup is physically closed/discarded, not
returned to an ordinary pool; driver adapters must prove that path. Async Drop
cannot await: default migration connections are dedicated, and pool integration
must supply discard-on-drop before it is supported. SQLite stale-claim recovery
and nontransactional effect recovery are separate assertions; clearing the
former does not declare the latter safe.

There is no exactly-once guarantee for arbitrary Rust, external IO, or
nontransactional scripts. Transactional guarantees apply only to supported
database effects on the selected session. Database retries, backoff loops, or
savepoints must never rerun user bodies automatically.

## SQL execution boundaries (#16/#17)

Prefer server/driver-native script execution where it preserves the declared
policy and returns complete errors. When a driver needs individual statements,
use shared dialect-aware lexical handling, never `split(';')`. Prevalidate the
whole script before dispatch so a later lexical error cannot partially execute
an otherwise rejected script. Preserve source byte ranges for diagnostics.

Initial grammar supports semicolons inside standard single-quoted strings,
double-quoted identifiers, line/block comments, PostgreSQL dollar-quoted bodies,
MySQL backtick identifiers and SQLite bracket identifiers. Honor the selected
engine's escaping/comment rules. Require PostgreSQL standard-conforming strings
and MySQL `NO_BACKSLASH_ESCAPES` for the initial grammar; set/verify these on the
dedicated session, and include relevant execution settings in artifact policy.
MySQL executable comments, client `DELIMITER`, psql backslash commands, COPY
stdin payloads, and SQL* client directives are rejected. SQLite trigger bodies
and MySQL compound routines are deferred unless a native parser/driver path
proves correct boundaries; never guess block nesting with semicolon counting.
PostgreSQL dollar-quoted contents remain one lexical unit, not client commands.

Reject explicit transaction control, session replacement, lock manipulation,
and reserved history writes through the provided script/raw-query surface;
unsupported forms fail explicitly. Direct or indirect control through trusted
stored code remains the author trust boundary described above. Driver tests
must demonstrate supported grammar and policy rejection; lexical splitting
alone does not establish atomicity, cancellation safety, or resumability.

## Immutable artifact closure and loading (#17)

Choose a versioned SHA-256 artifact checksum, identified as `rupa-artifact-v1`
with algorithm `sha256`. This is a design choice, not dependency installation;
repository approval for any new hashing dependency remains an implementation
prerequisite. Do not substitute a platform hasher or unstable Rust Debug output.

Canonical bytes use a fixed domain/version tag and length-prefixed fields.
Include stream, scheme, kind, canonical version, supported execution modes,
transaction policy, up/down availability and source bytes, SQL session settings,
snapshot coverage, and explicit helper/input files. Sort dialect variants and
input records by stable logical path, reject duplicate paths, and record every
variant as part of one artifact. The checksum is independent of the driver's
name, source directory's absolute path, host OS, enabled facade features, and
selected runtime dialect. Hash all declared variants, not only the local one.

Read UTF-8 files, remove one leading UTF-8 BOM and normalize CRLF to LF; retain
all other bytes, including whitespace and comments. Reject invalid UTF-8, lone
CR, path escape, duplicate logical names and symlink inputs initially. Logical
paths are normalized relative paths with `/`, never absolute host paths.
Descriptions and diagnostic root paths are excluded; executable input logical
paths are included, so changing an input path may change checksum despite the
artifact key remaining stable. Execute the normalized SQL bytes that were
hashed. No SQL formatting, comment removal, or language-level normalization,
except that discovery removes a leading description header (consecutive `--`
lines in SQL, `//!` lines in Rust) from the hashed inputs and from the executed
SQL. Compiled Rust keeps its inert `//!` lines; only its hashed bytes exclude them.

Rust is compiled code, not a filesystem runtime script. The migration directory
is the manifest (amended in #35): `<version>_<snake_name>.sql`,
`<version>_<snake_name>.rs`, or a `<version>_<snake_name>/` directory rooted at
`mod.rs`. The canonical file name version is the artifact key, and a Rust module
exports a unit struct named after the snake name in PascalCase. A directory
migration's files are its complete local closure. Unrecognized entries fail the
build. Hand-written catalogs remain available through `migration_catalog!`.
Constructors needing state must declare immutable input bytes; reject runtime
secrets/config as behavior inputs. Record source digests with the binary so
runtime metadata never rereads Rust source from disk to decide what compiled.

Hash Rust source and declared local helpers as normalized files. For crate
helpers, require a declared stable dependency-input record (package identity,
version and source revision/checksum) generated during the build. Explicitly
record compiler/toolchain and resolved feature configuration in diagnostic build
provenance, separately from the portable source checksum. Authors must declare
feature-sensitive executable helpers/variants explicitly in the artifact input
closure; changing those bytes/configuration changes checksum. Do not pretend a
source hash proves transitive semantic identity across compilers, dependency
resolution, runtime environment, or undeclared helper code. A closure audit and
fixtures are required; future automatic dependency analysis is deferred.

A planned Rust migration's identity is its plan (amended in #40): the checksum
covers the ordered operations, schema operations as canonical tagged records,
and it declares no source inputs. Refactoring its source without changing the
plan keeps the checksum. It may use shared entities: if one changes an applied
plan, the checksum mismatch blocks apply instead of the plan silently diverging,
and replacing the entity with the literal definition it produced restores the
recorded plan. Changing the canonical encoding requires a new checksum format.

Discovered scripted Rust migrations must not reach outside their own files: the build
helper rejects `crate::` paths (including inside macro tokens), `super::` above
the migration root, `#[path]`, out-of-line modules in single-file migrations and
every `include*!` in single-file migrations, and includes that leave a
directory migration or name an unhashed dotfile. Application entities and
repositories evolve after a migration is applied, and are neither hashed nor
frozen with it; migrations declare the row shapes they need locally. External
crates remain allowed as explicit dependencies, covered by the crate-helper rule
above rather than by the source checksum.

The directory drives code generation and rebuild tracking. The `rupa-build`
helper emits `cargo::rerun-if-changed` for the migration directory, which Cargo
scans recursively, so additions and removals rerun discovery rather than
silently changing a catalog. Rust module references ensure normal compilation tracking
as well. Do not rely solely on proc-macro `include_str!` of the previous file set
to detect new files. Clean and incremental builds must produce identical catalog
metadata. Keep generated artifacts in OUT_DIR; no runtime source traversal for
embedded catalogs.

Filesystem tooling may load SQL and its explicit manifest, applying the same
normalization and checksum rules. A filesystem source cannot load Rust modules
as scripts or claim an embedded Rust artifact is available without compiled
registration. Mixed catalogs require the compiled Rust registry plus declared
SQL inputs, with duplicate-key validation across both sources.

| Change / condition | Required result |
|---|---|
| SQL or Rust executable body edited | Checksum changes; applied identity mismatch blocks execution |
| Declared helper or policy edited | Checksum changes even if top-level body is identical |
| Different absolute checkout path / CRLF checkout | Same canonical checksum |
| Comment-only edit | Checksum changes; no semantic equivalence inference |
| Description-only edit | Same checksum and key |
| File rename with explicit key unchanged | Key unchanged; checksum may change if logical input path changed |
| Different driver or facade feature selection | Same checksum for the same complete declared closure |
| Declared executable feature/dependency input changes | Checksum changes; never hide a different body behind one artifact checksum |
| Add/remove migration file in the discovered directory | Build reruns; the catalog follows the directory, and unrecognized entries fail the build |
| Scripted Rust migration references application code (`crate::`, escaping `super::`/`include*!`/`#[path]`) | Build fails before the catalog exists |
| Planned migration's shared entity changes its applied plan | Checksum changes; apply refuses before running pending work |
| Planned migration's source refactored to the same plan | Same checksum |
| Schema operation the session dialect cannot express | Apply fails before acquiring the target |
| Different Rust source on disk after binary build | Binary retains its compiled source metadata; no runtime Rust reload |
| Undeclared helper behavior changes | Outside checksum guarantee; documented author error, never a semantic-integrity claim |

## Cross-cutting acceptance and handoff

The accepted earlier documents supply fresh, managed, unmanaged, snapshot and
adoption scenarios. Add strict/allowed late paths, nonmonotonic undo, missing
sources, byte-level checksum vectors, driver interoperability, and every
interruption window above to shared conformance. For concurrency, use independent
connections/processes including simultaneous first bootstrap; verify one user
execution and truthful history, not just a successful lock call. Use fake
sessions for planner/error state tests and real engines for SQL/locking claims.

Feature gates must prove: minimal sync with no runtime/erasure dependency,
async without implicit blocking, dynamic adaptation parity, macro consumers
without copied facade feature names, and each driver built independently. Each
implementation PR updates docs/tests with its runnable behavior; no new
standalone discovery document is a prerequisite after this proposal is approved.

Upwell integration must verify configuration access in Setup/PreBuild before
selecting a command phase, execute checks/apply before schema-dependent component
construction, and keep listeners/jobs stopped on failure. The current
`rupa-upwell` crate is a placeholder; this design makes no claim that the needed
Upwell API is already shipped. Identify any missing prerequisite in Upwell as
implementation evidence in #10, while keeping the standalone runner usable.
Speculative graph/reload preparation and inspect metadata execute no migrations.

On owner merge, close #14–#18 and #4 manually on the integration branch, update
#5–#11 to this contract, and retain epic #3 open. Those work items own executable
acceptance; approving the design does not itself claim backend conformance or
close the implementation issues.

Backend feasibility was checked against primary documentation on 2026-10-19:
[PostgreSQL session advisory locks](https://www.postgresql.org/docs/18/explicit-locking.html#ADVISORY-LOCKS),
[concurrent index transaction restriction](https://www.postgresql.org/docs/18/sql-createindex.html#SQL-CREATEINDEX-CONCURRENTLY),
[MySQL implicit DDL commits](https://dev.mysql.com/doc/refman/8.4/en/implicit-commit.html),
[MySQL named locks](https://dev.mysql.com/doc/refman/8.4/en/locking-functions.html),
[SQLite transaction lifetime](https://www.sqlite.org/lang_transaction.html), and
[SQLite VACUUM restrictions](https://www.sqlite.org/lang_vacuum.html).
The policy/exclusion choices here are Rupa design proposals, not features
promised by those engines or proof of a tested Rupa implementation.
