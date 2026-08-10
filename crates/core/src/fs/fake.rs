use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use async_trait::async_trait;

use super::FileSystem;

/// Test double. Keeps an in-memory tree, records every call, and can be told to fail.
#[derive(Default)]
pub struct FakeFileSystem {
    files: Mutex<BTreeMap<PathBuf, String>>,
    dirs: Mutex<BTreeSet<PathBuf>>,
    calls: Mutex<Vec<(String, PathBuf)>>,
    failures: Mutex<HashMap<PathBuf, ErrorKind>>,
}

impl FakeFileSystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a file, creating its parent directories.
    pub fn insert(&self, path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref().to_path_buf();
        self.make_dirs(path.parent());
        self.files
            .lock()
            .expect("files lock")
            .insert(path, contents.to_string());
    }

    /// Contents of a file, if present.
    pub fn contents(&self, path: impl AsRef<Path>) -> Option<String> {
        self.files
            .lock()
            .expect("files lock")
            .get(path.as_ref())
            .cloned()
    }

    /// Make every operation on `path` fail with `kind`.
    pub fn fail(&self, path: impl AsRef<Path>, kind: ErrorKind) {
        self.failures
            .lock()
            .expect("failures lock")
            .insert(path.as_ref().to_path_buf(), kind);
    }

    /// Every (operation, path) pair seen, in order.
    pub fn calls(&self) -> Vec<(String, PathBuf)> {
        self.calls.lock().expect("calls lock").clone()
    }

    fn record(&self, op: &str, path: &Path) {
        self.calls
            .lock()
            .expect("calls lock")
            .push((op.to_string(), path.to_path_buf()));
    }

    fn make_dirs(&self, dir: Option<&Path>) {
        let mut dirs = self.dirs.lock().expect("dirs lock");
        let mut cursor = dir;
        while let Some(d) = cursor {
            dirs.insert(d.to_path_buf());
            cursor = d.parent();
        }
    }

    /// The injected failure for `path`, as an `anyhow::Error` wrapping a real io error.
    fn injected(&self, op: &str, path: &Path) -> Option<anyhow::Error> {
        let kind = *self.failures.lock().expect("failures lock").get(path)?;
        Some(io_err(kind, op, path))
    }
}

fn io_err(kind: ErrorKind, op: &str, path: &Path) -> anyhow::Error {
    anyhow::Error::new(std::io::Error::new(kind, format!("{kind:?}")))
        .context(format!("{op} {}", path.display()))
}

#[async_trait]
impl FileSystem for FakeFileSystem {
    async fn read_to_string(&self, path: &Path) -> Result<String> {
        self.record("read_to_string", path);
        if let Some(e) = self.injected("reading", path) {
            return Err(e);
        }
        self.files
            .lock()
            .expect("files lock")
            .get(path)
            .cloned()
            .ok_or_else(|| io_err(ErrorKind::NotFound, "reading", path))
    }

    async fn write(&self, path: &Path, contents: &str) -> Result<()> {
        self.record("write", path);
        if let Some(e) = self.injected("writing", path) {
            return Err(e);
        }
        let parent_missing = match path.parent() {
            Some(p) => !self.dirs.lock().expect("dirs lock").contains(p),
            None => false,
        };
        if parent_missing {
            return Err(io_err(ErrorKind::NotFound, "writing", path));
        }
        self.files
            .lock()
            .expect("files lock")
            .insert(path.to_path_buf(), contents.to_string());
        Ok(())
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.record("create_dir_all", path);
        if let Some(e) = self.injected("creating", path) {
            return Err(e);
        }
        self.make_dirs(Some(path));
        Ok(())
    }

    async fn remove_file(&self, path: &Path) -> Result<()> {
        self.record("remove_file", path);
        if let Some(e) = self.injected("removing", path) {
            return Err(e);
        }
        self.files
            .lock()
            .expect("files lock")
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| io_err(ErrorKind::NotFound, "removing", path))
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        self.record("read_dir", path);
        if let Some(e) = self.injected("reading", path) {
            return Err(e);
        }
        if !self.dirs.lock().expect("dirs lock").contains(path) {
            return Err(io_err(ErrorKind::NotFound, "reading", path));
        }
        Ok(self
            .files
            .lock()
            .expect("files lock")
            .keys()
            .filter(|p| p.parent() == Some(path))
            .cloned()
            .collect())
    }

    async fn exists(&self, path: &Path) -> Result<bool> {
        self.record("exists", path);
        if let Some(e) = self.injected("stat", path) {
            return Err(e);
        }
        Ok(self.files.lock().expect("files lock").contains_key(path)
            || self.dirs.lock().expect("dirs lock").contains(path))
    }
}
