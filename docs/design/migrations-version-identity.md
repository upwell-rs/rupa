# Migration versions and artifact identity

Status: accepted design through owner-reviewed [PR #20](https://github.com/upwell-rs/rupa/pull/20),
merged into the migration integration branch on 2026-10-19. Not implemented.
Addresses [#12](https://github.com/upwell-rs/rupa/issues/12), under discovery
[#4](https://github.com/upwell-rs/rupa/issues/4) and epic
[#3](https://github.com/upwell-rs/rupa/issues/3).

## Requirements and scope

SQL files and compiled Rust migrations must share ordering and history rules.
Native static synchronous use must remain possible without async or dynamic
features. An applied identifier must never silently refer to different work.
These are requirements from the issue tree; the choices below are proposals.

This slice defines version syntax and artifact identity. It does not settle
checksum algorithms, snapshot selection, adoption, out-of-order permission,
reversal, or execution traits. Those remain in #13–#18. No crate, dependency,
public Rust signature, history schema, or release milestone is introduced.

## Recommended contract

Configure one explicit version scheme per catalog stream: numbered or UTC
timestamp. Do not infer the scheme from a filename: `20261019093000` is also
a valid positive integer. A stream is a logical history namespace, not a
promise of multi-stream execution or cross-stream dependencies in the first
release. Its identifier must remain stable once history exists.

Use a structured artifact key `(stream, kind, version)`, where kind distinguishes
incremental migrations from executable snapshots if snapshots are accepted in
#13. An up/down pair belongs to one artifact, not two identities. SQL versus
Rust, filename, description, and exported Rust type are not identity fields.
Adoption is a history event; it is not an executable artifact kind.

Within one stream, incremental versions must be unique across both source
languages and all execution modes. A snapshot may share an incremental version
because its kind differs; coverage still needs explicit artifact identities
under #13. The runner must not infer coverage from a numeric range alone.

Sources sharing a key are duplicates unless one catalog declaration explicitly
groups them as dialect variants of one artifact. A matching key or filename
never establishes that grouping automatically. The declaration syntax and
variant completeness rules belong to #5/#17; until those are agreed, reject
multiple sources at a key rather than infer variants.

### Numbered syntax

Accept ASCII decimal integers from `1` through `18446744073709551615` (`u64::MAX`).
Reject zero, signs, whitespace, separators, non-ASCII digits, overflow, and
leading zeroes. Store and display the canonical decimal spelling. Compare
numeric values: `2 < 10`, regardless of filename sort order.

Rejecting leading zeroes avoids multiple source spellings of the same key.
Padding can be a presentation concern later; it must not create another
identifier. Gaps are permitted: `1, 3, 10` does not imply missing artifacts.
Missing applied history is a separate validation policy under #6/#17.

For a numbered stream, propose generating one more than the largest catalog
version, starting at `1` for an empty catalog. Fail at `u64::MAX` and on a
source-path collision without writing a file. Generation is offline and does
not claim to reserve a number against other branches or installed history;
catalog/history validation remains mandatory before execution. Detailed
command behavior belongs to #9.

### Timestamp syntax

Accept exactly fourteen ASCII digits, `YYYYMMDDhhmmss`, representing a valid
Gregorian calendar date and UTC time. Years are `0001`–`9999`, hours `00`–`23`,
minutes and seconds `00`–`59`; leap seconds are rejected. The digits are the
canonical spelling. No local timezone, offset, fractional seconds, or suffix
is accepted. Date validation must reject impossible days, including invalid
leap days. Compare the validated calendar tuple in chronological order.

Use current UTC time to the second as the proposed generator default when
creating a new stream without a chosen scheme. Once configured, the stream's
scheme controls generation; numbered streams do not switch to timestamps.
The default is a convenience, not a dependency-order guarantee.

On a collision, generation fails without overwriting either source. The user
can wait for another UTC second or choose an unused valid timestamp explicitly.
Do not append a hidden counter, invent a future clock value, or renumber an
applied migration. Clock skew and branch merge order can still create late
migrations. Validate catalog uniqueness before execution; a generator's local
check cannot prevent a collision introduced by a later branch merge.

### Identity, edits, and renames

Record the stream's scheme with managed history and reject a catalog configured
with a different scheme before execution. A scheme change on an existing
stream requires a separate explicit conversion design; it is excluded here.

Compare an applied artifact key with its execution checksum, whose inputs and
algorithm identity are decided in #17. Reusing the key for changed executable
content must fail validation rather than overwrite history or run it again.
History must preserve enough identity to detect the mismatch even if the old
source is absent. Do not use a content hash as the only identity: an edit must
look like a mutation of existing work, not a newly pending migration.

A rename that preserves the explicit key does not by itself create pending
work. Filename and description are diagnostic metadata. Their inclusion in
the checksum remains a #17 decision, so this proposal does not promise that
every rename passes checksum validation. Changing a filename-derived version
changes the key and must not be treated as a rename of applied work.

An unapplied collision can be resolved by changing one artifact's version and
updating references before it reaches any managed target. Once applied to any
managed target, retain its key; write a new migration for a correction. An
explicit reset policy for disposable local databases is outside this slice.
Replacing an SQL source with Rust under the same key is also an artifact change
subject to checksum validation, not an exemption based on equivalent intended
effects.

## Examples for acceptance tests

These are required behaviors if this proposal is approved. They are design
vectors, not evidence of a working parser or planner.

| Scheme or context | Input | Expected result |
|---|---|---|
| Numbered | `2`, `10` | Accepted; `2` precedes `10` |
| Numbered | `1`, `3` | Accepted; no requirement for version `2` |
| Numbered | `0`, `01`, `+1`, `-1`, ` 1`, `1 `, `1_000`, `١` | Rejected |
| Numbered | `18446744073709551615` | Accepted |
| Numbered | `18446744073709551616` | Overflow rejected |
| Either scheme | `1.2`, `v1`, `1-alpha` | Rejected; no dotted or semver syntax |
| Timestamp | `20261019093000`, `20261019093001` | Accepted in that order |
| Timestamp | `20240229000000` | Accepted leap day |
| Timestamp | `20230229000000`, `19000229000000` | Invalid leap day rejected |
| Timestamp | `20000229000000` | Accepted leap day |
| Timestamp | `00010101000000`, `99991231235959` | Accepted year bounds |
| Timestamp | `00000101000000`, `20261301000000`, `20260431000000` | Invalid date rejected |
| Timestamp | `20261019240000`, `20261019096000`, `20261019093060` | Invalid time rejected |
| Timestamp | `2026101909300`, `202610190930000`, `20261019093000Z` | Wrong width or suffix rejected |
| Explicit scheme | `20261019093000` | Numbered integer or UTC timestamp according to configuration |
| Catalog | SQL and Rust incrementals at version `10` in one stream | Duplicate key rejected |
| Catalog | Explicitly grouped dialect variants of one artifact | One key; declaration and validation belong to #5/#17 |
| Catalog | Multiple ungrouped sources claiming dialect variation | Duplicate key rejected; matching versions do not imply grouping |
| Catalog | Incremental and snapshot at version `10` | Distinct keys if snapshots are approved |
| Applied history | Same key, changed executable checksum | Validation failure; no automatic replay |
| Applied history | Same key and checksum, different source path | Same artifact; path alone creates no pending work |
| Applied history | Source version changed from `10` to `11` | Different key; never silently rewrite applied `10` |
| Managed stream | Numbered history, timestamp catalog configuration | Scheme mismatch rejected |
| Late arrival | History `100, 102`, catalog adds `101` | Identity preserved; execution permission decided in #14 |
| Timestamp generation | Local clock behind an installed version | May produce a late artifact; never assume dependency order |
| Concurrent generation | Same timestamp key on two branches | Reject merged catalog collision before execution |

## Alternatives and consequences

| Alternative | Tradeoff and recommendation |
|---|---|
| Only sequential numbers | Simple and readable, but branches compete for the next number; retain as an explicit scheme |
| Only timestamps | Less coordination, but collisions and clock skew remain; retain as the proposed generator default, not the sole scheme |
| Infer a scheme per artifact | A fourteen-digit number is ambiguous; reject in favor of explicit stream configuration |
| Arbitrary padded numbers | Familiar filenames, but permits aliases; reject initially for one canonical spelling |
| Dotted or semver versions | Requires extra ordering and canonicalization rules; defer until a concrete migration use case needs it |
| Separate arbitrary immutable ID | Supports changing logical version independently, but adds another namespace and collision policy; defer in favor of a structured key |
| Content hash as identity | Makes edits look new and loses stable mutation detection; reject |
| Source path as identity | Makes filesystem organization affect migration history; reject |

For #5, parse versions using the configured scheme and reject duplicate keys
across SQL/Rust sources before producing a catalog. Preserve source locations
for diagnostics without making them identities. For #6, keep logical version
separate from actual installation rank. Never use the maximum installed
version as the complete set of applied identities. For #9, generators must
report collisions and never silently overwrite sources.

PR #20 records approval of the grammar, timestamp default, leading-zero
rejection, structured key, and rename rules. Carry this accepted contract into
#5/#6/#9 and align #13/#14/#17 with it. Issue #12 is complete; parent discovery
#4 remains open for the remaining decisions.
