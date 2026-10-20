mod discover;
mod error;
mod header;
mod isolation;
mod kind;
mod render;

use std::path::{Path, PathBuf};

use rupa_migrate::VersionScheme;

pub use error::{Error, Violation};

/// Migration directory used when [`Migrations::dir`] is not called, relative to
/// the package whose build script runs.
pub const DEFAULT_DIR: &str = "migrations";

/// Subdirectory of `OUT_DIR` that `include_migrations!` reads generated catalogs from.
const GENERATED_DIR: &str = "rupa-migrations";

/// Discovers one migration stream from a directory and generates its catalog.
///
/// Call it from the build script of the crate that should own the catalog, e.g.
/// a `db` crate whose `db/migrations` directory holds the files. The generated
/// catalog is engine-independent, so any crate can apply it:
///
/// ```no_run
/// // build.rs
/// fn main() -> Result<(), rupa_build::Error> {
///     rupa_build::Migrations::new("primary").compile()
/// }
/// ```
///
/// ```ignore
/// // src/lib.rs
/// pub mod migrations {
///     rupa::migrate::include_migrations!("primary");
/// }
/// ```
///
/// File names carry identity: `<version>_<snake_name>.sql`, `<version>_<snake_name>.rs`,
/// or a `<version>_<snake_name>/` directory whose `mod.rs` is the migration root and
/// whose other files are its private helpers. Leading `--` (SQL) or `//!` (Rust)
/// lines are the description and are excluded from the checksum, except a
/// `rupa: nontransactional` line, which declares the migration's policy.
/// Unrecognized entries and directives fail the build; dotfiles are ignored.
#[derive(Debug, Clone)]
pub struct Migrations {
    stream: String,
    dir: PathBuf,
    scheme: VersionScheme,
}

impl Migrations {
    /// Configures `stream` with [`DEFAULT_DIR`] and [`VersionScheme::Numbered`].
    pub fn new(stream: impl Into<String>) -> Self {
        Self {
            stream: stream.into(),
            dir: PathBuf::from(DEFAULT_DIR),
            scheme: VersionScheme::Numbered,
        }
    }

    /// Reads migrations from `dir`, relative to the build script's package unless absolute.
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = dir.into();

        self
    }

    /// Interprets file name versions with `scheme`.
    pub fn scheme(mut self, scheme: VersionScheme) -> Self {
        self.scheme = scheme;

        self
    }

    /// Generates the catalog into `OUT_DIR` and tells Cargo to rerun when the
    /// directory changes, including added or removed files.
    ///
    /// Failures are also reported as `cargo::error` so Cargo shows the readable
    /// message even when the build script returns the error from `main`.
    pub fn compile(self) -> Result<(), Error> {
        let result = self.write();

        if let Err(error) = &result {
            println!("cargo::error={error}");
        }

        result
    }

    fn write(&self) -> Result<(), Error> {
        let manifest =
            std::env::var_os("CARGO_MANIFEST_DIR").ok_or(Error::Env("CARGO_MANIFEST_DIR"))?;
        let out = std::env::var_os("OUT_DIR").ok_or(Error::Env("OUT_DIR"))?;
        let dir = Path::new(&manifest).join(&self.dir);
        let target = Path::new(&out).join(GENERATED_DIR);
        let file = target.join(format!("{}.rs", self.stream));

        println!("cargo::rerun-if-changed={}", dir.display());

        let source = self.generate(&dir)?;

        std::fs::create_dir_all(&target).map_err(|source| Error::Io {
            path: target,
            source,
        })?;

        std::fs::write(&file, source).map_err(|source| Error::Io { path: file, source })
    }

    fn generate(&self, dir: &Path) -> Result<String, Error> {
        let valid = !self.stream.is_empty()
            && self
                .stream
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');

        if !valid {
            return Err(Error::Stream(self.stream.clone()));
        }

        let artifacts = discover::discover(dir, self.scheme)?;

        render::catalog(&self.stream, self.scheme, &artifacts)
    }
}

#[cfg(test)]
mod tests;
