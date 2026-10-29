mod clock;
mod template;

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clock::timestamp;
use template::{description, imperative, pascal, planned};

use crate::{Catalog, MigrationKind, Version, VersionError, VersionScheme};

/// Where snapshots live inside the migration directory.
const SNAPSHOTS_DIR: &str = "snapshots";

/// The digits of a new numbered stream's first file name.
const NUMBERED_WIDTH: usize = 4;

/// The type name a migration must not take.
const RESERVED_TYPE: &str = "Self";

/// A new migration source to write. Offline: no target is read.
#[derive(Debug, Clone, Copy)]
pub struct Create<'a> {
    /// The catalog's migration directory, as rupa-build reads it.
    pub dir: &'a Path,
    /// The snake_case name after the version.
    pub name: &'a str,
    /// An incremental, or a snapshot of the newest incremental.
    pub kind: MigrationKind,
    /// How the body is written.
    pub authoring: Authoring,
    /// Whether to also write a down body. Snapshots have none.
    pub down: bool,
}

/// How a new migration's body is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authoring {
    /// A `.sql` script, with a `.down.sql` sibling for its down body.
    Sql,
    /// A Rust `PlannedMigration`: dialect-free data, in either mode.
    Planned,
    /// An imperative Rust `Migration` on sync sessions.
    Sync,
    /// An imperative Rust `Migration` on async sessions.
    Async,
}

/// What `create` wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// The new artifact's version.
    pub version: Version,
    /// The files written, the body first.
    pub paths: Vec<PathBuf>,
}

/// Why `create` wrote nothing.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CreateError {
    /// The name is not one rupa-build accepts.
    #[error(
        "migration name {0:?} must start with a lowercase ASCII letter and contain only lowercase letters, digits and underscores"
    )]
    Name(String),
    /// A snapshot was asked for with a down body.
    #[error("snapshots are never undone, so they have no down body")]
    SnapshotDown,
    /// A snapshot stands for the newest incremental, and there is none.
    #[error("the stream has no incremental for a snapshot to stand for")]
    NoIncremental,
    /// The numbered stream has no version left.
    #[error("numbered migration versions are exhausted after {0}")]
    Exhausted(Version),
    /// An artifact of that kind already has the version.
    #[error("version {0} already has an artifact of that kind")]
    Taken(Version),
    /// A file to write already exists; nothing is overwritten.
    #[error("{} already exists", .0.display())]
    Exists(PathBuf),
    /// The clock gave no valid UTC timestamp version.
    #[error(transparent)]
    Clock(#[from] VersionError),
    /// Reading the directory or writing a file failed.
    #[error("cannot write {}: {source}", path.display())]
    Io {
        /// The path.
        path: PathBuf,
        /// The cause.
        #[source]
        source: io::Error,
    },
}

/// Writes the files `create` describes, choosing the version from the
/// catalog and the directory: max + 1 for a numbered incremental, `now` for
/// a timestamped one, and the newest incremental for a snapshot. Never
/// overwrites a file; on failure, removes the files it wrote.
pub(super) fn create<M>(
    catalog: &Catalog<'_, M>,
    create: &Create<'_>,
    now: SystemTime,
) -> Result<Created, CreateError> {
    let snapshot = create.kind == MigrationKind::Snapshot;

    if !valid_name(create.name) {
        return Err(CreateError::Name(create.name.to_owned()));
    }

    if snapshot && create.down {
        return Err(CreateError::SnapshotDown);
    }

    let existing = Existing::read(catalog, create.dir)?;
    let version = existing.next(catalog.scheme(), create.kind, now)?;
    let stem = format!("{}_{}", existing.file_version(version), create.name);
    let files = files(create, &stem, version);

    write(&files)?;

    Ok(Created {
        version,
        paths: files.into_iter().map(|(path, _)| path).collect(),
    })
}

/// The versions the catalog and the directory already hold.
struct Existing {
    incrementals: Vec<Version>,
    snapshots: Vec<Version>,
    width: usize,
}

impl Existing {
    fn read<M>(catalog: &Catalog<'_, M>, dir: &Path) -> Result<Self, CreateError> {
        let scheme = catalog.scheme();
        let mut existing = Self {
            incrementals: catalog.incrementals().map(|e| e.version).collect(),
            snapshots: catalog.snapshots().map(|e| e.version).collect(),
            width: NUMBERED_WIDTH,
        };

        for (name, in_snapshots) in names(dir)? {
            let Some((digits, rest)) = name.split_once('_') else {
                continue;
            };
            let canonical = match scheme {
                VersionScheme::Numbered => digits.trim_start_matches('0'),
                VersionScheme::UtcTimestamp => digits,
            };
            let Ok(version) = scheme.parse(canonical) else {
                continue;
            };

            existing.width = existing.width.max(digits.len());

            if in_snapshots || rest.contains(".snapshot.") {
                existing.snapshots.push(version);
            } else if !rest.ends_with(".down.sql") {
                existing.incrementals.push(version);
            }
        }

        Ok(existing)
    }

    /// The version a new artifact of `kind` takes.
    fn next(
        &self,
        scheme: VersionScheme,
        kind: MigrationKind,
        now: SystemTime,
    ) -> Result<Version, CreateError> {
        let newest = self.incrementals.iter().max().copied();
        let version = match (kind, scheme) {
            (MigrationKind::Snapshot, _) => newest.ok_or(CreateError::NoIncremental)?,
            (MigrationKind::Incremental, VersionScheme::Numbered) => {
                successor(self.incrementals.iter().chain(&self.snapshots).max())?
            }
            (MigrationKind::Incremental, VersionScheme::UtcTimestamp) => {
                scheme.parse(&timestamp(now))?
            }
        };
        let taken = match kind {
            MigrationKind::Snapshot => &self.snapshots,
            MigrationKind::Incremental => &self.incrementals,
        };

        if taken.contains(&version) {
            return Err(CreateError::Taken(version));
        }

        Ok(version)
    }

    /// How `version` is spelled in a file name: padded to the directory's
    /// widest version, as numbered streams conventionally are.
    fn file_version(&self, version: Version) -> String {
        format!("{:0>width$}", version.to_string(), width = self.width)
    }
}

/// The numbered version after `newest`, or the first.
fn successor(newest: Option<&Version>) -> Result<Version, CreateError> {
    let next = match newest {
        Some(newest) => newest
            .to_string()
            .parse::<u64>()
            .expect("a numbered version is a u64")
            .checked_add(1)
            .ok_or(CreateError::Exhausted(*newest))?,
        None => 1,
    };

    Ok(VersionScheme::Numbered
        .parse(&next.to_string())
        .expect("a positive u64 is a numbered version"))
}

/// The file names in `dir` and its `snapshots/`, each marked with whether
/// it is in `snapshots/`. A missing directory holds none.
fn names(dir: &Path) -> Result<Vec<(String, bool)>, CreateError> {
    let mut names = Vec::new();

    for (dir, in_snapshots) in [(dir.to_path_buf(), false), (dir.join(SNAPSHOTS_DIR), true)] {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(CreateError::Io { path: dir, source }),
        };

        for entry in entries {
            let entry = entry.map_err(|source| CreateError::Io {
                path: dir.clone(),
                source,
            })?;

            names.extend(
                entry
                    .file_name()
                    .into_string()
                    .ok()
                    .map(|n| (n, in_snapshots)),
            );
        }
    }

    Ok(names)
}

/// The files to write and their contents, the body first.
fn files(create: &Create<'_>, stem: &str, version: Version) -> Vec<(PathBuf, String)> {
    let snapshot = create.kind == MigrationKind::Snapshot;
    let dir = if snapshot {
        create.dir.join(SNAPSHOTS_DIR)
    } else {
        create.dir.to_path_buf()
    };
    let description = description(create.name);
    let name = pascal(create.name);
    let mut files = Vec::new();

    match create.authoring {
        Authoring::Sql => {
            files.push((
                dir.join(format!("{stem}.sql")),
                format!("-- {description}\n"),
            ));

            if create.down {
                files.push((
                    dir.join(format!("{stem}.down.sql")),
                    format!("-- Undo migration {version}.\n"),
                ));
            }
        }
        Authoring::Planned => files.push((
            dir.join(format!("{stem}.rs")),
            planned(&description, &name, create.down),
        )),
        Authoring::Sync => files.push((
            dir.join(format!("{stem}.rs")),
            imperative("sync", "", &description, &name, create.down),
        )),
        Authoring::Async => files.push((
            dir.join(format!("{stem}.rs")),
            imperative("asynchronous", "async ", &description, &name, create.down),
        )),
    }

    files
}

/// Creates every file without overwriting one, removing those written if
/// any fails. Removal is best effort: the caller needs the original error,
/// not a cleanup failure.
fn write(files: &[(PathBuf, String)]) -> Result<(), CreateError> {
    let mut written: Vec<&Path> = Vec::new();
    let result = files.iter().try_for_each(|(path, contents)| {
        let io = |source| CreateError::Io {
            path: path.clone(),
            source,
        };

        if path.exists() {
            return Err(CreateError::Exists(path.clone()));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io)?;
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| match source.kind() {
                io::ErrorKind::AlreadyExists => CreateError::Exists(path.clone()),
                _ => io(source),
            })?;

        written.push(path);

        file.write_all(contents.as_bytes()).map_err(io)
    });

    if result.is_err() {
        for path in written {
            let _ = fs::remove_file(path);
        }
    }

    result
}

/// Whether rupa-build accepts `name` after a version.
fn valid_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && pascal(name) != RESERVED_TYPE
}
