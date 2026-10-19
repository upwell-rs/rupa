use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rupa_migrate::{Version, VersionScheme};

use super::Error;
use super::header::{self, Split};
use super::isolation::{self, Layout};

const SQL_MARKER: &str = "--";
const RUST_MARKER: &str = "//!";
const RUST_ROOT: &str = "mod.rs";

/// One discovered migration with the exact bytes its checksum covers.
#[derive(Debug)]
pub(super) struct Artifact {
    pub(super) version: Version,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) documented: bool,
    pub(super) body: Body,
}

/// How a discovered migration executes.
#[derive(Debug)]
pub(super) enum Body {
    /// A planned script; its single input is the executed SQL.
    Sql(Input),
    /// A compiled module rooted at `module`, with its whole file closure.
    Rust { module: PathBuf, inputs: Vec<Input> },
}

/// A logical input path and its hashed contents.
#[derive(Debug)]
pub(super) struct Input {
    pub(super) path: String,
    pub(super) contents: String,
}

/// The shape of a directory entry, decided by its name and file type.
enum Entry {
    Sql,
    RustFile,
    RustDir,
}

/// Reads every migration in `dir`, ordered by version.
pub(super) fn discover(dir: &Path, scheme: VersionScheme) -> Result<Vec<Artifact>, Error> {
    let mut seen: BTreeMap<Version, PathBuf> = BTreeMap::new();
    let mut artifacts = Vec::new();

    for path in entries(dir)? {
        let (version, name, entry) = parse(&path, scheme)?;

        if let Some(first) = seen.insert(version, path.clone()) {
            return Err(Error::Duplicate {
                version: version.to_string(),
                first,
                second: path,
            });
        }

        artifacts.push(load(&path, version, name, entry)?);
    }

    if artifacts.is_empty() {
        return Err(Error::Empty(dir.to_path_buf()));
    }

    artifacts.sort_by_key(|a| a.version);

    Ok(artifacts)
}

fn entries(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let io = |source| Error::Io {
        path: dir.to_path_buf(),
        source,
    };
    let mut paths = Vec::new();

    for entry in fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        let hidden = path
            .file_name()
            .is_some_and(|n| n.as_encoded_bytes().starts_with(b"."));

        if hidden {
            continue;
        }

        if fs::symlink_metadata(&path)
            .map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?
            .is_symlink()
        {
            return Err(Error::Symlink(path));
        }

        paths.push(path);
    }

    paths.sort();

    Ok(paths)
}

fn parse(path: &Path, scheme: VersionScheme) -> Result<(Version, String, Entry), Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let invalid = || Error::Name(path.to_path_buf());
    let (stem, entry) = if path.is_dir() {
        (file_name, Entry::RustDir)
    } else if let Some(stem) = file_name.strip_suffix(".sql") {
        (stem, Entry::Sql)
    } else if let Some(stem) = file_name.strip_suffix(".rs") {
        (stem, Entry::RustFile)
    } else {
        return Err(invalid());
    };
    let (digits, name) = stem.split_once('_').ok_or_else(invalid)?;
    let snake = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');

    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || !snake {
        return Err(invalid());
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

fn load(path: &Path, version: Version, name: String, entry: Entry) -> Result<Artifact, Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let (description, body) = match entry {
        Entry::Sql => {
            let source = read(path)?;
            let Split { description, body } = header::split(&source, SQL_MARKER);
            let input = Input {
                path: file_name.to_owned(),
                contents: body.to_owned(),
            };

            (description, Body::Sql(input))
        }
        Entry::RustFile => {
            let (description, input) = rust_input(path, file_name.to_owned(), Layout::File)?;
            let module = path.to_path_buf();

            (
                description,
                Body::Rust {
                    module,
                    inputs: vec![input],
                },
            )
        }
        Entry::RustDir => rust_dir(path, file_name)?,
    };
    let documented = description.is_some();
    let description = description.unwrap_or_else(|| name.replace('_', " "));

    Ok(Artifact {
        version,
        name,
        description,
        documented,
        body,
    })
}

fn rust_dir(dir: &Path, prefix: &str) -> Result<(Option<String>, Body), Error> {
    let module = dir.join(RUST_ROOT);
    let mut description = None;
    let mut inputs = Vec::new();

    if !module.is_file() {
        return Err(Error::Name(dir.to_path_buf()));
    }

    for (path, relative) in files(dir)? {
        let logical = format!("{prefix}/{}", relative.join("/"));

        if path.extension().is_some_and(|e| e == "rs") {
            let depth =
                relative.len() - usize::from(relative.last().is_some_and(|f| f == RUST_ROOT));
            let (header, input) = rust_input(
                &path,
                logical,
                Layout::Directory {
                    depth,
                    relative: &relative,
                },
            )?;

            if depth == 0 {
                description = header;
            }

            inputs.push(input);
        } else {
            let contents = read(&path)?;

            inputs.push(Input {
                path: logical,
                contents,
            });
        }
    }

    Ok((description, Body::Rust { module, inputs }))
}

fn rust_input(
    path: &Path,
    logical: String,
    layout: Layout<'_>,
) -> Result<(Option<String>, Input), Error> {
    let source = read(path)?;

    isolation::check(&logical, &source, layout)?;

    let Split { description, body } = header::split(&source, RUST_MARKER);
    let input = Input {
        path: logical,
        contents: body.to_owned(),
    };

    Ok((description, input))
}

/// Every regular file below `dir`, with its path components relative to `dir`.
fn files(dir: &Path) -> Result<Vec<(PathBuf, Vec<String>)>, Error> {
    let mut found = Vec::new();
    let mut pending = vec![(dir.to_path_buf(), Vec::new())];

    while let Some((current, relative)) = pending.pop() {
        for path in entries(&current)? {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| Error::Utf8(path.clone()))?
                .to_owned();
            let mut components = relative.clone();

            components.push(name);

            if path.is_dir() {
                pending.push((path, components));
            } else {
                found.push((path, components));
            }
        }
    }

    found.sort_by(|a, b| a.1.cmp(&b.1));

    Ok(found)
}

fn read(path: &Path) -> Result<String, Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    String::from_utf8(bytes).map_err(|_| Error::Utf8(path.to_path_buf()))
}
