use anyhow::{bail, Context, Result};
use tokio::fs;

use crate::exec::Executor;
use crate::spec::{slug, WorkloadSpec};
use crate::workloads::render::render;

pub use crate::workloads::paths::{unit_path, Paths};

/// Write the unit, reload systemd, and start the workload.
///
/// Idempotent: the same spec produces byte-identical output.
pub async fn apply(exec: &dyn Executor, paths: &Paths, spec: &WorkloadSpec) -> Result<()> {
    fs::create_dir_all(&paths.quadlet_dir)
        .await
        .with_context(|| format!("creating {}", paths.quadlet_dir.display()))?;

    let path = unit_path(paths, &spec.name);
    fs::write(&path, render(spec))
        .await
        .with_context(|| format!("writing {}", path.display()))?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;
    systemctl(exec, &["start".to_string(), slug(&spec.name)]).await?;

    Ok(())
}

/// Stop the workload, delete its unit, and reload systemd. Safe if absent.
pub async fn remove(exec: &dyn Executor, paths: &Paths, name: &str) -> Result<()> {
    let path = unit_path(paths, name);

    if path.exists() {
        systemctl(exec, &["stop".to_string(), slug(name)]).await?;
        fs::remove_file(&path)
            .await
            .with_context(|| format!("removing {}", path.display()))?;
        systemctl(exec, &["daemon-reload".to_string()]).await?;
    }

    Ok(())
}

async fn systemctl(exec: &dyn Executor, args: &[String]) -> Result<()> {
    let out = exec.run("systemctl", args).await?;
    if !out.success() {
        bail!("systemctl {} failed: {}", args.join(" "), out.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::spec::WorkloadSpec;
    use tempfile::tempdir;

    fn ok() -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn apply_writes_unit_reloads_and_starts() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply succeeds");

        let written = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("unit written");
        assert!(written.contains("Image=alpine"));

        let calls = fake.calls();
        assert_eq!(calls[0].1, vec!["daemon-reload".to_string()]);
        assert_eq!(calls[1].1, vec!["start".to_string(), "pbrain".to_string()]);
    }

    #[tokio::test]
    async fn apply_is_idempotent_for_the_same_spec() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("first apply");
        let first = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");
        apply(&fake, &paths, &spec).await.expect("second apply");
        let second = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn apply_fails_when_daemon_reload_fails() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect(
            "systemctl",
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "bad unit".into(),
            },
        );

        let spec = WorkloadSpec::new("pbrain", "alpine");
        let err = apply(&fake, &paths, &spec).await.unwrap_err();
        assert!(err.to_string().contains("daemon-reload"));
    }

    #[tokio::test]
    async fn remove_stops_unit_and_deletes_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply");
        remove(&fake, &paths, "pbrain").await.expect("remove");

        assert!(!unit_path(&paths, "pbrain").exists());
    }

    #[tokio::test]
    async fn remove_is_ok_when_unit_absent() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        remove(&fake, &paths, "never-existed")
            .await
            .expect("no error");
    }
}
