use std::io::ErrorKind;

use anyhow::{Context, Result};

use crate::exec::Executor;
use crate::fs::{io_error_kind, FileSystem};
use crate::workloads::paths::{spec_name_from_stem, unit_name, unit_path, Paths};
use crate::workloads::render::MANAGED_MARKER;

/// Runtime state of a workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadState {
    Running,
    Stopped,
    Failed,
    NotInstalled,
    Unknown,
}

impl WorkloadState {
    pub fn label(&self) -> &'static str {
        match self {
            WorkloadState::Running => "Running",
            WorkloadState::Stopped => "Stopped",
            WorkloadState::Failed => "Failed",
            WorkloadState::NotInstalled => "Not installed",
            WorkloadState::Unknown => "Unknown",
        }
    }
}

/// Current state of a workload. `NotInstalled` when no unit file exists.
pub async fn status(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    name: &str,
) -> Result<WorkloadState> {
    if !fsys.exists(&unit_path(paths, name)).await? {
        return Ok(WorkloadState::NotInstalled);
    }

    let out = exec
        .run("systemctl", &["is-active".to_string(), unit_name(name)])
        .await?;

    Ok(match out.stdout.trim() {
        "active" => WorkloadState::Running,
        "inactive" => WorkloadState::Stopped,
        "failed" => WorkloadState::Failed,
        _ => WorkloadState::Unknown,
    })
}

/// Names of every kuadrat-managed workload found in the quadlet directory.
///
/// "The directory does not exist" is an empty fleet; anything else — a permission error
/// above all — is an error. In a root-privileged tool the two must not look alike.
pub async fn list(fsys: &dyn FileSystem, paths: &Paths) -> Result<Vec<String>> {
    let dir = &paths.quadlet_dir;

    let entries = match fsys.read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if io_error_kind(&e) == Some(ErrorKind::NotFound) => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("listing {}", dir.display())),
    };

    let mut names = Vec::new();
    for path in entries {
        if path.extension().and_then(|e| e.to_str()) != Some("container") {
            continue;
        }
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(spec_name_from_stem)
        else {
            continue;
        };

        let content = match fsys.read_to_string(&path).await {
            Ok(content) => content,
            // Raced with a delete between listing and reading — not this call's problem.
            Err(e) if io_error_kind(&e) == Some(ErrorKind::NotFound) => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        if !content.starts_with(MANAGED_MARKER) {
            continue;
        }

        names.push(name.to_string());
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use crate::fs::local::LocalFileSystem;
    use crate::spec::WorkloadSpec;
    use crate::workloads::apply::apply;
    use tempfile::tempdir;

    fn out(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn status_is_not_installed_without_a_unit_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;

        let state = status(&fake, &fs, &paths, "absent").await.expect("status");
        assert_eq!(state, WorkloadState::NotInstalled);
    }

    #[tokio::test]
    async fn status_maps_systemctl_output() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;

        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));
        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("apply");

        assert_eq!(
            status(&fake, &fs, &paths, "pbrain").await.expect("status"),
            WorkloadState::Running
        );

        let fake2 = FakeExecutor::new();
        fake2.expect("systemctl", out("failed\n"));
        assert_eq!(
            status(&fake2, &fs, &paths, "pbrain").await.expect("status"),
            WorkloadState::Failed
        );

        let fake3 = FakeExecutor::new();
        fake3.expect("systemctl", out("inactive\n"));
        assert_eq!(
            status(&fake3, &fs, &paths, "pbrain").await.expect("status"),
            WorkloadState::Stopped
        );
    }

    #[tokio::test]
    async fn status_asks_systemd_about_the_prefixed_unit() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;
        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));

        apply(&fake, &fs, &paths, &WorkloadSpec::new("nginx", "alpine"))
            .await
            .expect("apply");
        status(&fake, &fs, &paths, "nginx").await.expect("status");

        let calls = fake.calls();
        assert_eq!(
            calls[2].1,
            vec!["is-active".to_string(), "kuadrat-nginx".to_string()]
        );
    }

    #[tokio::test]
    async fn list_returns_only_kuadrat_managed_units() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", out("active\n"));

        apply(&fake, &fs, &paths, &WorkloadSpec::new("alpha", "alpine"))
            .await
            .expect("apply alpha");
        apply(&fake, &fs, &paths, &WorkloadSpec::new("beta", "alpine"))
            .await
            .expect("apply beta");

        std::fs::write(paths.quadlet_dir.join("foreign.container"), "[Container]\n")
            .expect("write foreign unit");
        // A prefixed filename is not enough on its own — the marker decides.
        std::fs::write(
            paths.quadlet_dir.join("kuadrat-impostor.container"),
            "[Container]\n",
        )
        .expect("write impostor unit");

        let mut names = list(&fs, &paths).await.expect("list");
        names.sort();
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[tokio::test]
    async fn list_is_empty_when_the_quadlet_directory_does_not_exist() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;

        assert!(list(&fs, &paths)
            .await
            .expect("absent dir is empty")
            .is_empty());
    }

    #[tokio::test]
    async fn list_reports_an_unreadable_directory_instead_of_calling_it_empty() {
        let paths = Paths::rooted(std::path::Path::new("/root"));
        let fs = FakeFileSystem::new();
        fs.create_dir_all(&paths.quadlet_dir)
            .await
            .expect("create dir");
        fs.fail(&paths.quadlet_dir, ErrorKind::PermissionDenied);

        let err = list(&fs, &paths)
            .await
            .expect_err("a permission error is not an empty fleet");
        assert_eq!(io_error_kind(&err), Some(ErrorKind::PermissionDenied));
        assert!(err.to_string().contains("listing"), "{err}");
    }

    #[tokio::test]
    async fn list_reports_an_unreadable_unit_file() {
        let paths = Paths::rooted(std::path::Path::new("/root"));
        let fs = FakeFileSystem::new();
        let unit = paths.quadlet_dir.join("kuadrat-alpha.container");
        fs.insert(&unit, &format!("{MANAGED_MARKER}\n"));
        fs.fail(&unit, ErrorKind::PermissionDenied);

        let err = list(&fs, &paths)
            .await
            .expect_err("an unreadable unit is not a missing unit");
        assert_eq!(io_error_kind(&err), Some(ErrorKind::PermissionDenied));
    }
}
