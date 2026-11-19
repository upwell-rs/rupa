# rupa-example-db

## Role

Example persistence crate: entities live here, and `migrations/` holds the
discovered migration directory.

## Usage

`build.rs` compiles two streams: `primary`, numbered, from `migrations/`, and
`reporting`, versioned by UTC timestamps, from `reporting/`. `src/lib.rs`
exposes their catalogs as `rupa_example_db::migrations` and
`rupa_example_db::reporting`. `cargo nextest run -p rupa-example-db` applies
both to SQLite databases, `reporting` through the operator commands.

## Internal role

Shows all three migration shapes: a SQL file, a single-file Rust migration and
a directory migration with a private helper. Each Rust migration declares the
`notes` shape it needs locally instead of using the crate's current `Note`.
Migrations 1, 3 and 4 are reversible, through a `.down.sql` file, a scripted
`down` and a down plan; migration 2 is irreversible, so a revert reaching it is
refused before anything runs. `snapshots/0003_notes_with_slugs.sql` creates the
state migrations 1 to 3 leave, so a fresh database runs it and then only
migration 4; the tests compare that path with a full replay.

`reporting/` shows the same authoring with timestamp versions: a SQL file
with a `.down.sql` sibling, and an imperative Rust migration with a `down`
method, written by the `create` command.

## Feature flags

None.
