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
struct named `<SnakeName>` implementing `Migration<S>`), or a
`<version>_<snake_name>/` directory whose `mod.rs` is the root and whose other
files are private helpers. Leading `--` or `//!` lines are the description and
are excluded from the checksum. Anything else fails the build; dotfiles are
ignored.

## Internal role

Discovery reuses `rupa-migrate`'s version grammar, so names are validated at
build time with the same rules the runtime applies. Output goes to
`OUT_DIR/rupa-migrations/<stream>.rs`; the build reruns when the directory
changes, so added or removed files never leave a stale catalog.

Rust migrations are parsed with `syn` and must stay inside their own files: no
`crate::` paths, no `super::` above the migration root, no `#[path]`, no
out-of-line modules in single-file migrations, and no `include*!` outside the
migration directory. Otherwise a later change to shared application code would
silently change an applied migration that its checksum still reports as
unchanged. Migrations declare the row shapes they need locally instead.

## Feature flags

None.
