# rupa-build

## Role

Build-script helpers that turn RUPA project layouts into generated code. The
first helper discovers a migration directory and generates its static catalog.

## Usage

Add `rupa-build` as a build dependency of the crate that owns the migrations,
typically a `db` crate with a `db/migrations/` directory:

```rust,ignore
// db/build.rs
fn main() -> Result<(), rupa_build::Error> {
    rupa_build::Migrations::new("primary").compile()
}

// db/src/lib.rs
pub mod migrations {
    rupa::migrate::include_migrations!("primary");
}

// app/src/main.rs: any crate can apply the engine-independent catalog
db::migrations::catalog()?.apply(SqliteExecutor::open("app.db")?)?;
```

`.dir("…")` points at another directory, relative to the build script's
package, and `.scheme(VersionScheme::UtcTimestamp)` switches the version
grammar. The migrations can equally live in a dedicated `db-migrations` crate;
the generated catalog never assumes where it is applied.

Entries are `<version>_<snake_name>.sql`, `<version>_<snake_name>.rs` (a unit
struct named `<SnakeName>` implementing `Migration<S>` or `PlannedMigration`), or
a `<version>_<snake_name>/` directory whose `mod.rs` is the root and whose other
files are private helpers. Leading `--` or `//!` lines are the description and
are excluded from the checksum. From the first line starting with `rupa:`, the
header is YAML frontmatter instead (`-- rupa:` then `--   policy: nontransactional`,
or the flow form `-- rupa: { policy: nontransactional }`); its settings are part
of the checksum. Unknown keys or values, malformed YAML, and frontmatter in a
scripted directory migration's `.rs` helpers fail the build; a planned directory
migration's helpers and non-Rust helpers are never header-parsed. Anything else fails the build; dotfiles are ignored.

## Internal role

Frontmatter deserializes with `serde-saphyr` into one
`#[serde(default, deny_unknown_fields)]` struct (`frontmatter::Frontmatter`). A
new setting is one field there; valid values come from its type's
`Deserialize` impl (e.g. `rupa_migrate::Policy`), so parse errors list them
without a separate keyword table.

Discovery reuses `rupa-migrate`'s version grammar, so names are validated at
build time with the same rules the runtime applies. Output goes to
`OUT_DIR/rupa-migrations/<stream>.rs`; the build reruns when the directory
changes, so added or removed files never leave a stale catalog.

Rust migration roots are parsed with `syn`. One whose type implements
`PlannedMigration` is wrapped in `Planned<..>` and declares no inputs: its plan
is its checksummed identity, so it may use shared entities. Implementing both
traits fails the build. Scripted migrations must stay inside their own files: no
`crate::` paths (attribute and macro arguments included), no `super::` above the
migration root, no `#[path]`, no out-of-line modules or `include*!` in
single-file migrations, and no includes that leave a directory migration or name
an unhashed dotfile. Otherwise a later change to shared application code would
silently change an applied migration that its checksum still reports as
unchanged. Migrations declare the row shapes they need locally instead.

## Feature flags

None.
