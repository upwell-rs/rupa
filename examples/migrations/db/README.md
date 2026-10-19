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

## Feature flags

None.
