use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rupa_migrate::{Policy, Version, VersionScheme};

mod name;

use super::Error;
use super::header::{self, Header};
use super::isolation::{self, Layout};
use super::kind::{self, Kind};
use super::render::pascal;
use name::Entry;

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
    pub(super) policy: Policy,
    /// Whether it is a snapshot rather than an incremental.
    pub(super) snapshot: bool,
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

/// Reads every migration in `dir`, ordered by version with a snapshot after
/// the incremental of its version. Snapshots are `.snapshot.sql` or
/// `.snapshot.rs` files or the entries of `snapshots/`; down scripts are
/// `.down.sql` files or the `.sql` files of `down/`, each paired with the
/// SQL migration of its version and name.
pub(super) fn discover(dir: &Path, scheme: VersionScheme) -> Result<Vec<Artifact>, Error> {
    let mut seen: BTreeMap<(Version, bool), PathBuf> = BTreeMap::new();
    let mut downs: BTreeMap<Version, (String, PathBuf)> = BTreeMap::new();
    let mut artifacts = Vec::new();

    for (path, place) in name::places(dir)? {
        let (version, name, entry) = name::parse(&path, scheme, place)?;
        let earlier = match entry {
            Entry::SqlDown => downs
                .insert(version, (name.clone(), path.clone()))
                .map(|(_, first)| first),
            _ => seen.insert((version, entry.snapshot()), path.clone()),
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
        let artifact = artifacts.iter_mut().find(|artifact| {
            artifact.version == version && artifact.name == name && !artifact.snapshot
        });

        match artifact.map(|artifact| &mut artifact.body) {
            Some(Body::Sql { down, .. }) => *down = Some(down_input(&path)?),
            Some(Body::Rust { .. }) => return Err(Error::RustDown(path)),
            None => return Err(Error::UnpairedDown(path)),
        }
    }

    artifacts.sort_by_key(|a| (a.version, a.snapshot));

    Ok(artifacts)
}

pub(super) fn entries(dir: &Path) -> Result<Vec<PathBuf>, Error> {
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

fn load(path: &Path, version: Version, name: String, entry: Entry) -> Result<Artifact, Error> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Utf8(path.to_path_buf()))?;
    let root = match entry {
        Entry::RustDir(_) => path.join(RUST_ROOT),
        _ => path.to_path_buf(),
    };
    let snapshot = entry.snapshot();
    let kind = match entry {
        Entry::RustFile(_) | Entry::RustDir(_) => rust_kind(&root, file_name, &name)?,
        Entry::Sql(_) | Entry::SqlDown => Kind::default(),
    };
    if snapshot && kind.down {
        return Err(Error::SnapshotDown(root));
    }

    let (header, body) = match entry {
        Entry::Sql(_) | Entry::SqlDown => {
            let source = read(path)?;
            let split = header::split(&source, SQL_MARKER);
            let up = Input::new(file_name.to_owned(), split.body, path)?;

            (split.header(path)?, Body::Sql { up, down: None })
        }
        Entry::RustFile(_) | Entry::RustDir(_) if kind.planned => planned_body(&root, path)?,
        Entry::RustFile(_) => {
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
        Entry::RustDir(_) => rust_dir(path, file_name, kind)?,
    };
    let documented = header.description.is_some();
    let description = header.description.unwrap_or_else(|| name.replace('_', " "));

    Ok(Artifact {
        version,
        name,
        description,
        documented,
        policy: header.frontmatter.unwrap_or_default().policy,
        snapshot,
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
