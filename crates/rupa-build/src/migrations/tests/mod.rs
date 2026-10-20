use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

mod discovery;
mod frontmatter;
mod header;
mod isolation;

/// A disposable migration directory populated from `(relative path, contents)` pairs.
struct Fixture(PathBuf);

impl Fixture {
    fn new(files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "rupa-build-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        std::fs::create_dir_all(&root).expect("create fixture root");

        for (path, contents) in files {
            let path = root.join(path);

            std::fs::create_dir_all(path.parent().expect("fixture file has a parent"))
                .expect("create fixture directory");
            std::fs::write(path, contents).expect("write fixture file");
        }

        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn discover(&self, scheme: VersionScheme) -> Result<Vec<discover::Artifact>, Error> {
        discover::discover(self.path(), scheme)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove fixture directory");
    }
}
