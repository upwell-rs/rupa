# rupa-example-db

## Role

Example persistence crate: entities live here, and `migrations/` holds the
discovered migration directory.

## Usage

`build.rs` calls `rupa_build::Migrations::new("primary").compile()`, and
`src/lib.rs` exposes the generated catalog as `rupa_example_db::migrations`.
`cargo nextest run -p rupa-example-db` applies it to an in-memory SQLite database.

## Internal role

Shows all three migration shapes: a SQL file, a single-file Rust migration and
a directory migration with a private helper. Each Rust migration declares the
`notes` shape it needs locally instead of using the crate's current `Note`.
Migrations 1, 3 and 4 are reversible, through a `.down.sql` file, a scripted
`down` and a down plan; migration 2 is irreversible, so a revert reaching it is
refused before anything runs. `snapshots/0003_notes_with_slugs.sql` creates the
state migrations 1 to 3 leave, so a fresh database runs it and then only
migration 4; the tests compare that path with a full replay.

## Feature flags

None.
