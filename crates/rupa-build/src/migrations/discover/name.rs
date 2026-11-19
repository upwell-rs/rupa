//! Where a migration entry lives and what its name declares.

use std::path::{Path, PathBuf};

use rupa_migrate::{Version, VersionScheme};

use super::super::Error;
use super::super::render::pascal;
use super::entries;

const DOWN_SUFFIX: &str = ".down.sql";
const SNAPSHOT_SUFFIX: &str = ".snapshot";
const SNAPSHOTS_DIR: &str = "snapshots";
const DOWN_DIR: &str = "down";
const RESERVED_TYPE: &str = "Self";

/// Which part of the migration directory an entry is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    /// The directory itself.
    Root,
    /// `snapshots/`, whose entries are snapshots.
    Snapshots,
    /// `down/`, whose `.sql` files are down scripts.
    Down,
}

/// The shape of a directory entry, decided by its name, file type and
/// place; the flag marks a snapshot.
#[derive(Debug, Clone, Copy)]
pub(super) enum Entry {
    Sql(bool),
    SqlDown,
    RustFile(bool),
    RustDir(bool),
}

impl Entry {
    /// Whether the entry is a snapshot.
    pub(super) fn snapshot(self) -> bool {
        match self {
            Self::Sql(snapshot) | Self::RustFile(snapshot) | Self::RustDir(snapshot) => snapshot,
            Self::SqlDown => false,
        }
    }
}

/// Every entry of `dir` with its place, the contents of `snapshots/` and
/// `down/` included.
pub(super) fn places(dir: &Path) -> Result<Vec<(PathBuf, Place)>, Error> {
    let mut found = Vec::new();

    for path in entries(dir)? {
        let place = match path.file_name().and_then(|n| n.to_str()) {
            Some(SNAPSHOTS_DIR) if path.is_dir() => Place::Snapshots,
            Some(DOWN_DIR) if path.is_dir() => Place::Down,
            _ => {
                found.push((path, Place::Root));

                continue;
            }
        };

        found.extend(entries(&path)?.into_iter().map(|path| (path, place)));
    }

    Ok(found)
}

/// The version, name and shape `path` declares in `place`.
pub(super) fn parse(
    path: &Path,
    scheme: VersionScheme,
    place: Place,
) -> Result<(Version, String, Entry), Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let (stem, entry) = shape(path, file_name, place)?;
    let invalid = || Error::Name(path.to_path_buf());
    let (digits, name) = stem.split_once('_').ok_or_else(invalid)?;
    let snake = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');

    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || !snake {
        return Err(invalid());
    }

    if pascal(name) == RESERVED_TYPE {
        return Err(Error::ReservedName(path.to_path_buf()));
    }

    let canonical = match scheme {
        VersionScheme::Numbered => digits.trim_start_matches('0'),
        VersionScheme::UtcTimestamp => digits,
    };
    let version = scheme.parse(canonical).map_err(|source| Error::Version {
        path: path.to_path_buf(),
        source,
    })?;

    Ok((version, name.to_owned(), entry))
}

/// The stem and shape of `file_name` in `place`.
fn shape<'a>(path: &Path, file_name: &'a str, place: Place) -> Result<(&'a str, Entry), Error> {
    let snapshot = place == Place::Snapshots;
    let invalid = || Error::Name(path.to_path_buf());

    if place == Place::Down {
        return match file_name.strip_suffix(".sql") {
            Some(stem) if !path.is_dir() && !stem.ends_with(".down") => Ok((stem, Entry::SqlDown)),
            _ => Err(invalid()),
        };
    }

    if path.is_dir() {
        return Ok((file_name, Entry::RustDir(snapshot)));
    }

    if let Some(stem) = file_name.strip_suffix(DOWN_SUFFIX) {
        if snapshot || stem.ends_with(SNAPSHOT_SUFFIX) {
            return Err(Error::SnapshotDown(path.to_path_buf()));
        }

        return Ok((stem, Entry::SqlDown));
    }

    let (stem, rust) = if let Some(stem) = file_name.strip_suffix(".sql") {
        (stem, false)
    } else if let Some(stem) = file_name.strip_suffix(".rs") {
        (stem, true)
    } else {
        return Err(invalid());
    };
    let (stem, snapshot) = match stem.strip_suffix(SNAPSHOT_SUFFIX) {
        Some(stem) if !snapshot => (stem, true),
        Some(_) => return Err(invalid()),
        None => (stem, snapshot),
    };
    let entry = if rust {
        Entry::RustFile(snapshot)
    } else {
        Entry::Sql(snapshot)
    };

    Ok((stem, entry))
}
