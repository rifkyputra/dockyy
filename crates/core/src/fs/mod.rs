pub mod fake;
pub mod local;

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;

/// The single seam through which `core` touches the filesystem.
///
/// Sibling of [`crate::exec::Executor`]: that trait abstracts *process* execution,
/// this one abstracts *storage*. Both are needed for a remote transport to be purely
/// additive — an SSH transport supplies its own implementation of each, and the engine
/// does not change. Implementations: `LocalFileSystem` (real disk), `FakeFileSystem`
/// (tests), and later a remote filesystem for the fleet driver.
///
/// Like `Executor`, no method takes a host parameter.
#[async_trait]
pub trait FileSystem: Send + Sync {
    async fn read_to_string(&self, path: &Path) -> Result<String>;
    async fn write(&self, path: &Path, contents: &str) -> Result<()>;
    async fn create_dir_all(&self, path: &Path) -> Result<()>;
    async fn remove_file(&self, path: &Path) -> Result<()>;
    /// Full paths of the entries directly inside `path`, in unspecified order.
    async fn read_dir(&self, path: &Path) -> Result<Vec<PathBuf>>;
    async fn exists(&self, path: &Path) -> Result<bool>;
}

/// The `std::io::ErrorKind` underlying an error, if there is one.
///
/// Implementations must preserve the original `std::io::Error` in the context chain so
/// callers can tell "the directory is absent" from "I am not allowed to read it".
pub fn io_error_kind(err: &anyhow::Error) -> Option<std::io::ErrorKind> {
    err.downcast_ref::<std::io::Error>().map(|e| e.kind())
}

#[cfg(test)]
mod tests {
    use super::{io_error_kind, FileSystem};
    use crate::fs::fake::FakeFileSystem;
    use crate::fs::local::LocalFileSystem;
    use std::io::ErrorKind;
    use std::path::Path;
    use tempfile::tempdir;

    #[tokio::test]
    async fn local_filesystem_round_trips_a_file() {
        let dir = tempdir().expect("tempdir");
        let fs = LocalFileSystem;
        let nested = dir.path().join("a/b");
        fs.create_dir_all(&nested).await.expect("create_dir_all");

        let file = nested.join("unit.container");
        fs.write(&file, "hello").await.expect("write");
        assert!(fs.exists(&file).await.expect("exists"));
        assert_eq!(fs.read_to_string(&file).await.expect("read"), "hello");
        assert_eq!(
            fs.read_dir(&nested).await.expect("read_dir"),
            vec![file.clone()]
        );

        fs.remove_file(&file).await.expect("remove");
        assert!(!fs.exists(&file).await.expect("exists"));
    }

    #[tokio::test]
    async fn local_filesystem_read_dir_reports_not_found() {
        let dir = tempdir().expect("tempdir");
        let fs = LocalFileSystem;
        let err = fs
            .read_dir(&dir.path().join("absent"))
            .await
            .expect_err("missing dir errors");
        assert_eq!(io_error_kind(&err), Some(ErrorKind::NotFound));
    }

    #[tokio::test]
    async fn fake_filesystem_round_trips_and_records_calls() {
        let fs = FakeFileSystem::new();
        let file = Path::new("/q/unit.container");
        fs.create_dir_all(Path::new("/q")).await.expect("mkdir");
        fs.write(file, "body").await.expect("write");

        assert!(fs.exists(file).await.expect("exists"));
        assert_eq!(fs.read_to_string(file).await.expect("read"), "body");
        assert_eq!(fs.contents(file).as_deref(), Some("body"));
        assert_eq!(
            fs.read_dir(Path::new("/q")).await.expect("read_dir"),
            vec![file.to_path_buf()]
        );

        let calls = fs.calls();
        assert_eq!(calls[0].0, "create_dir_all");
        assert_eq!(calls[1].0, "write");
        assert_eq!(calls[1].1, file.to_path_buf());
    }

    #[tokio::test]
    async fn fake_filesystem_reports_not_found_and_injected_errors() {
        let fs = FakeFileSystem::new();
        let missing = fs
            .read_dir(Path::new("/nope"))
            .await
            .expect_err("missing dir errors");
        assert_eq!(io_error_kind(&missing), Some(ErrorKind::NotFound));

        fs.create_dir_all(Path::new("/q")).await.expect("mkdir");
        fs.fail(Path::new("/q"), ErrorKind::PermissionDenied);
        let denied = fs
            .read_dir(Path::new("/q"))
            .await
            .expect_err("injected failure");
        assert_eq!(io_error_kind(&denied), Some(ErrorKind::PermissionDenied));
    }
}
