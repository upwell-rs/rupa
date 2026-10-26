use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rupa_migrate::{Policy, Version, VersionScheme};

use super::Error;
use super::header::{self, Header};
use super::isolation::{self, Layout};
use super::kind::{self, Kind};
use super::render::pascal;

const SQL_MARKER: &str = "--";
const DOWN_SUFFIX: &str = ".down.sql";
const RUST_MARKER: &str = "//!";
const RUST_ROOT: &str = "mod.rs";
const RESERVED_TYPE: &str = "Self";

/// One discovered migration with the exact bytes its checksum covers.
#[derive(Debug)]
pub(super) struct Artifact {
    pub(super) version: Version,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) documented: bool,
    pub(super) policy: Policy,
    pub(super) body: Body,
}

/// How a discovered migration executes.
#[derive(Debug)]
pub(super) enum Body {
    /// A planned script; its inputs are the executed SQL and, when a
    /// `.down.sql` file pairs with it, the down script.
    Sql { up: Input, down: Option<Input> },
    /// A compiled module rooted at `module`. A scripted migration carries its
    /// whole file closure; a planned one is identified by its plan instead.
    /// `kind.down` marks a scripted migration whose impl defines `down`.
    Rust {
        module: PathBuf,
        inputs: Vec<Input>,
        kind: Kind,
    },
}

/// A logical input path and its hashed contents.
#[derive(Debug)]
pub(super) struct Input {
    pub(super) path: String,
    pub(super) contents: String,
}

impl Input {
    /// Applies the runtime catalog's input rules at build time, so a malformed
    /// entry fails the build instead of every later `catalog()` call.
    fn new(path: String, contents: &str, source: &Path) -> Result<Self, Error> {
        if path.split('/').any(|c| c.contains([':', '\\'])) {
            return Err(Error::InputPath(path));
        }

        if contents.replace("\r\n", "\n").contains('\r') {
            return Err(Error::CarriageReturn(source.to_path_buf()));
        }

        Ok(Self {
            path,
            contents: contents.to_owned(),
        })
    }
}

/// The shape of a directory entry, decided by its name and file type.
enum Entry {
    Sql,
    SqlDown,
    RustFile,
    RustDir,
}

/// Reads every migration in `dir`, ordered by version, each `.down.sql`
/// file paired with the SQL migration of its version and name.
pub(super) fn discover(dir: &Path, scheme: VersionScheme) -> Result<Vec<Artifact>, Error> {
    let mut seen: BTreeMap<Version, PathBuf> = BTreeMap::new();
    let mut downs: BTreeMap<Version, (String, PathBuf)> = BTreeMap::new();
    let mut artifacts = Vec::new();

    for path in entries(dir)? {
        let (version, name, entry) = parse(&path, scheme)?;
        let earlier = match entry {
            Entry::SqlDown => downs
                .insert(version, (name.clone(), path.clone()))
                .map(|(_, first)| first),
            _ => seen.insert(version, path.clone()),
        };

        if let Some(first) = earlier {
            return Err(Error::Duplicate {
                version: version.to_string(),
                first,
                second: path,
            });
        }

        if !matches!(entry, Entry::SqlDown) {
            artifacts.push(load(&path, version, name, entry)?);
        }
    }

    if artifacts.is_empty() {
        return Err(Error::Empty(dir.to_path_buf()));
    }

    for (version, (name, path)) in downs {
        let artifact = artifacts
            .iter_mut()
            .find(|artifact| artifact.version == version && artifact.name == name);

        match artifact.map(|artifact| &mut artifact.body) {
            Some(Body::Sql { down, .. }) => *down = Some(down_input(&path)?),
            Some(Body::Rust { .. }) => return Err(Error::RustDown(path)),
            None => return Err(Error::UnpairedDown(path)),
        }
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
    } else if let Some(stem) = file_name.strip_suffix(DOWN_SUFFIX) {
        (stem, Entry::SqlDown)
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

fn load(path: &Path, version: Version, name: String, entry: Entry) -> Result<Artifact, Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let root = match entry {
        Entry::RustDir => path.join(RUST_ROOT),
        _ => path.to_path_buf(),
    };
    let kind = match entry {
        Entry::RustFile | Entry::RustDir => rust_kind(&root, file_name, &name)?,
        Entry::Sql | Entry::SqlDown => Kind::default(),
    };
    let (header, body) = match entry {
        Entry::Sql | Entry::SqlDown => {
            let source = read(path)?;
            let split = header::split(&source, SQL_MARKER);
            let up = Input::new(file_name.to_owned(), split.body, path)?;

            (split.header(path)?, Body::Sql { up, down: None })
        }
        Entry::RustFile | Entry::RustDir if kind.planned => planned_body(&root, path)?,
        Entry::RustFile => {
            let (header, input) = rust_input(path, file_name.to_owned(), Layout::File)?;
            let module = path.to_path_buf();

            (
                header,
                Body::Rust {
                    module,
                    inputs: vec![input],
                    kind,
                },
            )
        }
        Entry::RustDir => rust_dir(path, file_name, kind)?,
    };
    let documented = header.description.is_some();
    let description = header.description.unwrap_or_else(|| name.replace('_', " "));

    Ok(Artifact {
        version,
        name,
        description,
        documented,
        policy: header.frontmatter.unwrap_or_default().policy,
        body,
    })
}

/// The input of a `.down.sql` file: its body after any comment header,
/// which may not declare frontmatter.
fn down_input(path: &Path) -> Result<Input, Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let source = read(path)?;
    let split = header::split(&source, SQL_MARKER);

    if split.frontmatter.is_some() {
        return Err(Error::DownFrontmatter(path.to_path_buf()));
    }

    Input::new(file_name.to_owned(), split.body, path)
}

fn rust_dir(dir: &Path, prefix: &str, kind: Kind) -> Result<(Header, Body), Error> {
    let module = dir.join(RUST_ROOT);
    let mut root = None;
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
                root = Some(header);
            } else if header.frontmatter.is_some() {
                return Err(Error::HelperFrontmatter(path));
            }

            inputs.push(input);
        } else {
            let contents = read(&path)?;

            inputs.push(Input::new(logical, &contents, &path)?);
        }
    }

    let header = root.ok_or_else(|| Error::Name(dir.to_path_buf()))?;

    Ok((
        header,
        Body::Rust {
            module,
            inputs,
            kind,
        },
    ))
}

/// How the Rust migration rooted at `root` executes. A directory without
/// `mod.rs` is left to [`rust_dir`] to reject.
fn rust_kind(root: &Path, logical: &str, name: &str) -> Result<Kind, Error> {
    if !root.is_file() {
        return Ok(Kind::default());
    }

    kind::kind(logical, &read(root)?, &pascal(name))
}

/// A planned Rust migration: its root's header, and no hashed inputs.
fn planned_body(root: &Path, module: &Path) -> Result<(Header, Body), Error> {
    let source = read(root)?;
    let header = header::split(&source, RUST_MARKER).header(root)?;
    let module = if module.is_dir() {
        root.to_path_buf()
    } else {
        module.to_path_buf()
    };

    Ok((
        header,
        Body::Rust {
            module,
            inputs: Vec::new(),
            kind: Kind {
                planned: true,
                down: false,
            },
        },
    ))
}

fn rust_input(path: &Path, logical: String, layout: Layout<'_>) -> Result<(Header, Input), Error> {
    let source = read(path)?;

    isolation::check(&logical, &source, layout)?;

    let split = header::split(&source, RUST_MARKER);
    let input = Input::new(logical, split.body, path)?;

    Ok((split.header(path)?, input))
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
