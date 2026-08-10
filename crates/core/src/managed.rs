//! The ownership guard shared by everything that writes marker-tagged files
//! (unit files, Caddy fragments): refuse to overwrite or delete a file kuadrat
//! did not write, so a hand-authored config is never silently clobbered.

use std::path::Path;

use anyhow::{bail, Result};

use crate::fs::FileSystem;

/// `Ok(true)` when the file exists and starts with `marker`, `Ok(false)` when
/// it is absent, and an error when a file is present that does not carry the
/// marker (so kuadrat did not write it).
pub(crate) async fn ensure_owned(
    fsys: &dyn FileSystem,
    path: &Path,
    marker: &str,
    action: &str,
) -> Result<bool> {
    if !fsys.exists(path).await? {
        return Ok(false);
    }

    let existing = fsys.read_to_string(path).await?;
    if !existing.starts_with(marker) {
        bail!(
            "refusing to {action} {}: the file exists but does not start with `{marker}`, \
             so kuadrat did not write it; resolve the drift by hand",
            path.display()
        );
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::fake::FakeFileSystem;
    use std::path::Path;

    const MARKER: &str = "# kuadrat-managed: true";

    #[tokio::test]
    async fn absent_file_is_not_owned() {
        let fsys = FakeFileSystem::new();
        assert!(!ensure_owned(&fsys, Path::new("/x/a"), MARKER, "overwrite")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_file_with_the_marker_is_owned() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/x/a", "# kuadrat-managed: true\nrest\n");
        assert!(ensure_owned(&fsys, Path::new("/x/a"), MARKER, "overwrite")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn a_foreign_file_is_refused() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/x/a", "hand written\n");
        let err = ensure_owned(&fsys, Path::new("/x/a"), MARKER, "remove")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("remove"), "message was: {msg}");
        assert!(msg.contains("/x/a"), "message was: {msg}");
    }
}
