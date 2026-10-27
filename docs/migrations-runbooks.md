# Operating migrations

Runbooks for the [operator commands](migrations.md#operator-commands). The
examples use the example application's command line
(`app <stream> <command>`). Your application embeds the same commands under
its own name. Every command prints `--format json` for scripts, and its
[exit status](migrations.md#reports-and-exit-statuses) says what happened.

## Checks before a deployment

- In CI, run `app primary validate --engine postgres` for every engine you
  deploy to. It is offline: no database, no credentials.
- Against staging, then production, run `app primary plan`. Exit `0` means
  `apply` would run exactly the listed steps. Exit `3` means something blocks
  it, and `blockers` says what, with the version or attempt involved. Planned
  steps show their SQL. Rust bodies are opaque until they run.
- `status` and `plan` take no lock and write nothing, so they are safe while
  the application runs.

## Rolling deployments

Run `apply` once per target, as its own deployment step (a job, an init
container or a release phase), before new application code starts. If several
replicas apply on startup, one runs and the others wait or fail with exit `4`
(`busy`). Either way, the migrations run exactly once.

During a rolling deployment, old and new code use the same schema at the
same time, so every migration must work with both:

1. **Expand.** Add tables, nullable columns or columns with defaults, and new
   indexes (on PostgreSQL, `CREATE INDEX CONCURRENTLY` in a
   `nontransactional` migration). Deploy code that writes both shapes.
2. **Migrate data.** Backfill in a migration, or in application code for
   large tables. Long backfills don't belong in one migration body: a body
   cannot resume from a checkpoint.
3. **Contract.** Only after no running code reads the old shape, drop or
   rename it in a later release.

Merged branches can leave a pending version below an installed one. Strict
`apply` refuses it (`late`). Run `apply --allow-late` only once you have
checked that it is independent of everything installed after it. Rupa
cannot check that for you.

## Busy targets and stale SQLite claims (exit 4)

On PostgreSQL and MySQL, a live session holds the target's lock: another
runner is still applying. Wait for it. The lock ends with that session, so a
crashed runner never leaves it behind.

SQLite has no session locks. A runner that dies keeps its durable owner claim,
and every later run fails with `busy`.

1. Confirm the runner has stopped: no process has the database open, or its
   pod or container has terminated. While it runs, its claim is valid.
2. Release the claim:
   `app primary recover release-claim --reason "pod web-7 terminated" --evidence "kubectl events"`.
3. Run `apply`. An interrupted transactional attempt is now recorded as
   rolled back, and `apply` exits `6` (`reconciled`). Run `apply` again.

## Reconciled attempts (exit 6)

A transactional attempt was interrupted, so its changes and its completion
never committed. `apply` recorded it as rolled back and ran nothing. Run
`apply` again: the artifact runs from the start, as a new attempt.

## Uncertain attempts (exit 5)

A `nontransactional` body was interrupted or failed, and some of its
statements may have run. Nothing else runs until an operator settles it.

1. Run `app primary status`. Note the unresolved attempt's id, version,
   direction (`up`, or `down` undoing an installation) and source file.
2. Confirm the runner that started it has stopped.
3. Compare the database with what that body does. For example, check whether
   the index exists and is valid, or whether the column was added.
4. Settle it in one of two ways:
   - Finish the remaining statements by hand, then record the attempt as
     complete:
     `app primary recover completed 7 --reason "index exists and is valid" --evidence "INC-1234"`.
     The artifact is installed with provenance `recovered`. For a `down`
     attempt, the installation it undoes becomes inactive.
   - Undo the partial effects by hand, then record the attempt as restored:
     `app primary recover restored 7 --reason "dropped the partial index"`.
     The next `apply` runs the artifact again. You may fix the artifact first,
     because a restored attempt does not pin its checksum.
5. Run `apply`.

`recover` refuses an assertion it can check is wrong: an unknown or resolved
attempt, or a changed or missing artifact for `completed`. Those exit `3`
and record nothing. Rupa cannot check whether the database effects match your
assertion. The journal keeps your reason and evidence on the attempt. Never
edit `_rupa_migrations` by hand.

## Adopting an existing database

`apply` refuses an existing database Rupa does not manage. It exits `3`, and
the blocker says `unmanaged`. To bring the database under management:

1. Pick the newest version whose schema and required data the database
   already holds.
2. Prove it. On a scratch database, run `app primary apply --through <version>`,
   then compare a schema dump of the scratch database with one of the target.
   Compare the reference data your migrations insert, too.
3. Record the assertion:
   `app primary adopt --through <version> --reason "legacy schema matches 0042" --evidence "schema diff in INC-1234"`.
   The report lists the incrementals recorded as covered. None of their
   bodies ran.
4. Run `status`, then `apply` to run the rest.

Adoption is permanent. It refuses a fresh target (run `apply` instead) and a
managed one. An incremental added later at or below the boundary is
`uncovered` and refused: write a new migration above the boundary instead.
If the assertion was wrong, restore the database from a backup. Rupa has no
command that removes an adoption.

## Rollback limits

`revert` undoes the newest installations, newest first, with their authored
down bodies. Preview it first with `revert --last 1 --dry-run`. It refuses
the whole revert before running anything when:

- an installation it would undo has no down body (`irreversible`),
- an installation it keeps sits among those it would undo (`interleaved`; the
  selection is not the newest installations),
- it asks for more installations than exist (`exceeds`),
- it would reach below a snapshot or adoption boundary (`boundary`). Covered
  incrementals never ran individually, so nothing can undo them.

A down body only does what its author wrote. Dropping a column it added does
not bring back the data that was lost when the up body dropped or rewrote
something. Use backups or point-in-time recovery for that. Each undo commits
on its own. If a down body fails, the earlier undos stay committed, and the
report lists what remains installed. A nontransactional down body can leave
an uncertain attempt, which you recover like an `up` attempt. An undone
version becomes pending, and late if newer installations remain.

Prefer a forward fix, a new migration, for anything already in production.
Keep `revert` for development and for a release that has just failed while
the old code is still running.

## New environments

A fresh database starts from the newest snapshot at or below the target
version, then applies the incrementals above it. Use `apply --full-replay`
to replay every incremental instead, for example to check that a replay
produces the same schema as the snapshot. Only a journal that has never
recorded an installation or coverage starts from a snapshot.
