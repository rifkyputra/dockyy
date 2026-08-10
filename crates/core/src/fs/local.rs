use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::fs;

use super::FileSystem;

/// Reads and writes the local disk.
///
/// The only place in the crate permitted to touch `tokio::fs`, exactly as `exec::local`
/// is the only place permitted to touch `tokio::process::Command`.
pub struct LocalFileSystem;

#[async_trait]
impl FileSystem for LocalFileSystem {
    async fn read_to_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path)
            .await
            .with_context(|| format!("reading {}", path.display()))
    }

    async fn write(&self, path: &Path, contents: &str) -> Result<()> {
        fs::write(path, contents)
            .await
            .with_context(|| format!("writing {}", path.display()))
    }

    async fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path)
            .await
            .with_context(|| format!("creating {}", path.display()))
    }

    async fn remove_file(&self, path: &Path) -> Result<()> {
        fs::remove_file(path)
            .await
            .with_context(|| format!("removing {}", path.display()))
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>> {
        let mut entries = fs::read_dir(path)
            .await
            .with_context(|| format!("reading {}", path.display()))?;

        let mut out = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("reading {}", path.display()))?
        {
            out.push(entry.path());
        }
        Ok(out)
    }

    async fn exists(&self, path: &Path) -> Result<bool> {
        match fs::metadata(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).with_context(|| format!("stat {}", path.display())),
        }
    }
}
