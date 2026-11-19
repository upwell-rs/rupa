# Adoption, snapshots, and coverage

Status: accepted design through owner-reviewed [PR #29](https://github.com/upwell-rs/rupa/pull/29),
merged into the migration integration branch on 2026-10-19. Implemented by
[#63](https://github.com/upwell-rs/rupa/issues/63); usage is in
[the migrations guide](../migrations.md#initializing-from-a-snapshot).
Addresses [#13](https://github.com/upwell-rs/rupa/issues/13), under discovery
[#4](https://github.com/upwell-rs/rupa/issues/4) and epic
[#3](https://github.com/upwell-rs/rupa/issues/3). Builds on the accepted
[version and identity contract](migrations-version-identity.md).

## Scope and vocabulary

Both SQL and compiled Rust migrations use the same history semantics. Neither
async nor dynamic support is required by this contract. This proposal defines
fresh initialization and explicit adoption, without specifying public Rust
signatures, storage encoding, checksum algorithms, or supported engine modes.
Those remain in #5–#8/#16/#17, with initial release inclusion decided in #18.

| Concept | Meaning | History claim |
|---|---|---|
| Incremental artifact | Changes the prior managed state | Executed only after successful execution is recorded |
| Executable snapshot | Creates the state represented by an explicit set of incrementals on a fresh target | Snapshot executed; listed incrementals covered by that snapshot |
| Adoption event | Operator declares an unmanaged target equivalent to an explicit set of incrementals | Listed incrementals adopted, never individually executed |

Use separate operations and report labels for adoption and snapshot execution.
The unqualified word “baseline” must not select between them. A snapshot has
the artifact key `(stream, snapshot, version)`, and its version must equal its
coverage boundary. There is at most one snapshot per boundary in a stream;
explicitly grouped dialect variants remain one artifact. Adoption is a history
event, not an executable artifact. Down steps do not have separate artifact
keys.

## Coverage is an immutable set

Represent coverage as an explicit manifest of incremental artifact keys and
their execution checksum identities, including the checksum algorithm. The
manifest belongs to one stream and its configured version scheme. Its encoding
and checksum closure are decided in #17. A numeric boundary alone is never
sufficient to establish membership or artifact equivalence.

For this proposal, a coverage boundary `N` must contain every incremental in
the reviewed catalog at versions at or below `N`, and none above `N`. Gaps in
numbered versions remain legal. Persist the manifest with history so that later
catalog changes cannot silently enlarge the set. Include the manifest in the
snapshot artifact's checksum inputs; altering it changes the snapshot artifact.
Adoption records the exact manifest that the operator approved.

The boundary describes planning and reversal limits. It does not mean every
integer up to `N` ran or exists. A later artifact at an unlisted version below
`N` is uncovered even if its version is numerically inside the boundary.

Persist the provenance of each coverage claim: the executed snapshot or the
adoption event. A covered/adopted artifact's matching checksum demonstrates
which source was represented by the claim, not proof that its body ran on the
target. Reports must preserve this distinction after later migrations apply.

## Target classification and ownership

Classify a selected, explicitly owned target before planning initialization:

- Fresh: no migration history or attempts and no user objects in the owned
  namespace; only documented engine-owned objects may be excluded.
- Managed: valid migration history for the selected stream and scheme.
- Unmanaged: user objects exist but no valid migration history does.
- Uncertain: incomplete attempts, incompatible history, unreadable ownership
  scope, or an interrupted initialization prevents safe classification.

Absence of the history table does not prove freshness. An empty existing user
table still makes a target unmanaged. Namespace boundaries and engine-owned
object exclusions must be explicit driver contracts, with conformance tests;
if the driver cannot prove the classification, reject initialization.

Recheck freshness and history after acquiring exclusive migration access and
before any initialization writes. Driver exclusion must cover history creation,
snapshot execution, and completion recording as one runner-owned session under
#16. This excludes competing migration runners; preventing arbitrary external
DDL requires operator-controlled ownership and access. Do not claim that a
runner lock alone excludes unrelated clients.

## Fresh replay and snapshot selection

On a fresh target, propose preferring the greatest compatible snapshot boundary
at or below the requested target. Compatibility requires a matching stream,
scheme, supported engine/dialect and execution policy, and an exact verified
coverage manifest against the supplied incremental catalog. A target between
artifact versions is permitted: choose only artifacts at or below the target.

Reject a malformed or checksum-inconsistent catalog rather than bypassing the
error by picking another snapshot. If the valid catalog has no eligible
snapshot, replay the incrementals. Provide explicit full-replay selection for
operators and equivalence fixtures; it requires the full incremental chain.
No snapshot body or imperative Rust body runs during plan/status inspection.

After a snapshot succeeds, record that snapshot as executed and its manifest
members as covered, then apply incrementals above its boundary through the
requested target. Never also execute the covered incremental bodies. The
snapshot and its covered records must have one durable completion outcome;
the supported atomic or uncertain-recovery protocol is decided in #16/#8.

For a managed target, continue from its recorded artifact set. Never apply a
newer snapshot on top of managed state, even if its version is eligible. A
snapshot added to the catalog does not make an existing incremental path
pending or replace its history.

## Explicit adoption

Ordinary apply refuses unmanaged targets. Adoption requires a separate explicit
operator action naming the stream, scheme, coverage boundary, and reviewed
manifest. The operator must assert schema and required data equivalence; no
generic entity comparison or version number proves it. Optional application
verification may strengthen that assertion but cannot replace explicit intent.

Under exclusive migration access, verify that no managed history or uncertain
attempt exists, validate the manifest, and durably record the adoption event
and its adopted members. Record the assertion and operator-supplied reason in
the audit report. Adoption executes no historical artifact bodies or snapshot
body. It necessarily writes migration history; read-only inspection does not.

Recommend allowing adoption only for nonempty unmanaged targets. Fresh targets
must use replay or snapshot execution so their schema is actually created.
Managed targets cannot be re-adopted to erase discrepancies or skip pending
work. Adoption never resets an uncertain attempt. After adoption, apply only
the tail above the boundary, subject to the same validation as other managed
targets.

## Late artifacts, pruning, and reversal

A newly added incremental at or below a snapshot/adoption boundary that is absent
from the persisted manifest is not covered. Propose rejecting automatic apply
with a diagnostic naming that artifact and the boundary. Write a corrective
migration above the boundary, or use a separately designed explicit recovery
procedure; ordinary out-of-order permission in #14 must not retroactively
rewrite coverage. A checksum mismatch for a manifest member also blocks apply.

Retain all historical sources initially, including covered/adopted incrementals
and executed snapshots. A snapshot is not permission to prune its source
chain: older targets may still need replay, validation, or reversal. Missing
recorded or manifest-referenced artifacts fail validation. A future retention
policy must define compatible target populations, archived manifests, and
offline validation before permitting removal. No pruning command is proposed.

Allow authored reversal of tail migrations only within the guarantees agreed
in #14/#16. Reject a target below a snapshot/adoption boundary and reject undo
of the snapshot/adoption event itself in the initial contract. Incremental
down bodies cannot reverse work that was never individually executed. Reaching
the boundary by reversing the tail preserves the original coverage and audit
events. Destructive reset or restoring a backup is a separate operator action,
not an inferred down migration or deletion of audit history.

## Scenarios for acceptance tests

Use numbered examples below; the same rules apply to UTC timestamp streams.
These are required test vectors if approved, not execution evidence.

| Target and catalog | Requested action | Expected result |
|---|---|---|
| Fresh; incrementals `1, 2, 3`, no snapshot | Apply through `3` | Execute `1, 2, 3`; record each as executed |
| Fresh; snapshot `2` covers `1, 2`; tail `3` | Apply through `3` | Execute snapshot `2`, mark `1, 2` covered, execute `3` |
| Fresh; snapshots `2, 4`, complete manifests; incrementals through `5` | Apply through `3` | Choose snapshot `2`, then execute `3`; snapshot `4` is ineligible |
| Fresh; only snapshot `4`; incrementals `1, 2, 3, 4` | Apply through `3` | Replay `1, 2, 3` |
| Fresh; valid snapshot `2` and full chain through `3` | Explicit full replay | Execute incrementals only; no snapshot execution |
| Nonempty unmanaged; no history table | Ordinary apply | Refuse; no automatic snapshot or adoption |
| Nonempty unmanaged, operator asserts equivalence through `2` | Adopt reviewed manifest, then apply through `3` | Record `1, 2` adopted without their bodies; execute only `3` |
| Fresh target | Adopt through `2` | Reject; adoption does not construct missing schema |
| Managed by incremental `1, 2`; catalog adds snapshot `3` | Apply through `3` | Execute incremental `3`; never run snapshot `3` |
| Snapshot/adoption through `3`, manifest contains only `1, 3`; later catalog adds `2` | Apply | Reject uncovered historical `2`; never silently mark it covered |
| Snapshot/adoption through `3`, manifest contains only `1, 2`; later catalog adds `3` | Apply | Reject uncovered artifact at the boundary; it is not part of the tail |
| Manifest records `2` checksum A; catalog supplies checksum B | Validate/apply | Fail checksum validation; preserve history |
| History or manifest references a removed source | Validate/apply | Fail missing-artifact validation; snapshot existence does not authorize pruning |
| Snapshot/adoption through `2`, tail `3` executed | Revert to `2` | Authored tail reversal if supported; retain coverage/adoption |
| Snapshot/adoption through `2`, tail `3` executed | Revert to `1` | Reject boundary crossing before running down bodies |
| Uncertain snapshot attempt | Apply or adopt | Refuse blind replay/adoption; require explicit recovery under #8/#16 |
| Two runners observe a fresh target | Initialize concurrently | Serialize, reclassify under exclusion; never duplicate snapshot execution |

For shared conformance (#11), compare full replay through `N` with
snapshot-plus-tail through `N` on fresh isolated targets. Compare owned schema,
constraints and required data effects, including Rust data transformations and
reference data. Exclude engine-generated identifiers only through explicit
fixture normalization. Equality of maximum versions or table names is not
sufficient. Run equivalence per supported engine/dialect and execution mode;
do not execute arbitrary migration bodies as a production inspection method.

## Alternatives and handoff

| Alternative | Decision proposed and rationale |
|---|---|
| One automatic baseline operation | Reject; confuses creating state with asserting existing state |
| Fresh means no history table | Reject; risks creating over unmanaged user objects |
| Coverage means every version below `N` | Reject; silently includes late additions and changed artifacts |
| Apply newer snapshots to managed targets | Reject; snapshots describe fresh initialization, not incremental transitions |
| Infer down from covered incrementals | Reject; those bodies did not execute and may not invert the snapshot |
| Prune once a snapshot exists | Defer; would strand older targets and lose validation evidence |
| Automatically verify arbitrary adopted schema | Defer; application equivalence includes data and external assumptions |

Following approval in PR #29, #5 owns immutable snapshot metadata and manifests; #6 owns
distinct executed/covered/adopted history and planning; #7/#16 own target
classification and exclusion; #8 owns execution/recovery and reverse boundaries;
#9 owns explicit adoption, replay selection, and reports; #11 owns equivalence
and interruption fixtures. #18 must explicitly decide which of these
capabilities ship initially. Issue #13 is complete; remaining discovery
decisions are consolidated in [the migration contract](migrations-contract.md).
