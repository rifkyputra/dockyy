use anyhow::{Context, Result};
use tokio::fs;

use crate::exec::Executor;
use crate::spec::slug;
use crate::workloads::paths::{unit_path, Paths};
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
pub async fn status(exec: &dyn Executor, paths: &Paths, name: &str) -> Result<WorkloadState> {
    if !unit_path(paths, name).exists() {
        return Ok(WorkloadState::NotInstalled);
    }

    let out = exec
        .run("systemctl", &["is-active".to_string(), slug(name)])
        .await?;

    Ok(match out.stdout.trim() {
        "active" => WorkloadState::Running,
        "inactive" => WorkloadState::Stopped,
        "failed" => WorkloadState::Failed,
        _ => WorkloadState::Unknown,
    })
}

/// Names of every kuadrat-managed workload found in the quadlet directory.
pub async fn list(paths: &Paths) -> Result<Vec<String>> {
    let mut names = Vec::new();

    let mut entries = match fs::read_dir(&paths.quadlet_dir).await {
        Ok(entries) => entries,
        Err(_) => return Ok(names),
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("reading {}", paths.quadlet_dir.display()))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("container") {
            continue;
        }
        let content = match fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(_) => continue,
        };
        if !content.starts_with(MANAGED_MARKER) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            names.push(stem.to_string());
        }
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::spec::WorkloadSpec;
    use crate::workloads::apply::{apply, Paths};
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

        let state = status(&fake, &paths, "absent").await.expect("status");
        assert_eq!(state, WorkloadState::NotInstalled);
    }

    #[tokio::test]
    async fn status_maps_systemctl_output() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());

        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));
        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply");

        assert_eq!(
            status(&fake, &paths, "pbrain").await.expect("status"),
            WorkloadState::Running
        );

        let fake2 = FakeExecutor::new();
        fake2.expect("systemctl", out("failed\n"));
        assert_eq!(
            status(&fake2, &paths, "pbrain").await.expect("status"),
            WorkloadState::Failed
        );

        let fake3 = FakeExecutor::new();
        fake3.expect("systemctl", out("inactive\n"));
        assert_eq!(
            status(&fake3, &paths, "pbrain").await.expect("status"),
            WorkloadState::Stopped
        );
    }

    #[tokio::test]
    async fn list_returns_only_kuadrat_managed_units() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));

        apply(&fake, &paths, &WorkloadSpec::new("alpha", "alpine"))
            .await
            .expect("apply alpha");
        apply(&fake, &paths, &WorkloadSpec::new("beta", "alpine"))
            .await
            .expect("apply beta");

        std::fs::write(paths.quadlet_dir.join("foreign.container"), "[Container]\n")
            .expect("write foreign unit");

        let mut names = list(&paths).await.expect("list");
        names.sort();
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }
}
