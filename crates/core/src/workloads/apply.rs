use std::path::Path;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::spec::WorkloadSpec;
use crate::workloads::render::{render, MANAGED_MARKER};

pub use crate::workloads::paths::{unit_name, unit_path, Paths};

/// Write the unit, reload systemd, and start the workload.
///
/// Idempotent: the same spec produces byte-identical output. Refuses to touch a unit file
/// kuadrat does not own.
pub async fn apply(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    spec: &WorkloadSpec,
) -> Result<()> {
    let unit = render(spec)?;
    let path = unit_path(paths, &spec.name);

    ensure_owned(fsys, &path, "overwrite").await?;

    fsys.create_dir_all(&paths.quadlet_dir).await?;
    fsys.write(&path, &unit).await?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;
    systemctl(exec, &["start".to_string(), unit_name(&spec.name)]).await?;

    Ok(())
}

/// Stop the workload, delete its unit, and reload systemd. Safe if absent.
///
/// Refuses to delete a unit file kuadrat does not own.
pub async fn remove(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    name: &str,
) -> Result<()> {
    let path = unit_path(paths, name);

    if !ensure_owned(fsys, &path, "remove").await? {
        return Ok(());
    }

    systemctl(exec, &["stop".to_string(), unit_name(name)]).await?;
    fsys.remove_file(&path).await?;
    systemctl(exec, &["daemon-reload".to_string()]).await?;

    Ok(())
}

/// `Ok(true)` when the unit exists and carries kuadrat's marker, `Ok(false)` when it is
/// absent, and an error when a file is there that kuadrat did not write.
///
/// This is the one ownership rule; `apply`, `remove`, and `list` all defer to the marker
/// rather than each deciding for itself. The design says drift is reported, never silently
/// overwritten — this is where that is enforced.
async fn ensure_owned(fsys: &dyn FileSystem, path: &Path, action: &str) -> Result<bool> {
    if !fsys.exists(path).await? {
        return Ok(false);
    }

    let existing = fsys.read_to_string(path).await?;
    if !existing.starts_with(MANAGED_MARKER) {
        bail!(
            "refusing to {action} {}: the file exists but does not start with `{MANAGED_MARKER}`, \
             so kuadrat did not write it; resolve the drift by hand",
            path.display()
        );
    }

    Ok(true)
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
    use crate::fs::fake::FakeFileSystem;
    use crate::fs::local::LocalFileSystem;
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
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec)
            .await
            .expect("apply succeeds");

        let written = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("unit written");
        assert!(written.contains("Image=alpine"));

        let calls = fake.calls();
        assert_eq!(calls[0].1, vec!["daemon-reload".to_string()]);
        assert_eq!(
            calls[1].1,
            vec!["start".to_string(), "kuadrat-pbrain".to_string()]
        );
    }

    #[tokio::test]
    async fn apply_is_idempotent_for_the_same_spec() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("first apply");
        let first = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");
        apply(&fake, &fs, &paths, &spec)
            .await
            .expect("second apply");
        let second = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn apply_fails_when_daemon_reload_fails() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect(
            "systemctl",
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "bad unit".into(),
            },
        );

        let spec = WorkloadSpec::new("pbrain", "alpine");
        let err = apply(&fake, &fs, &paths, &spec).await.unwrap_err();
        assert!(err.to_string().contains("daemon-reload"));
    }

    #[tokio::test]
    async fn remove_stops_unit_and_deletes_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("apply");
        remove(&fake, &fs, &paths, "pbrain").await.expect("remove");

        assert!(!unit_path(&paths, "pbrain").exists());
        let calls = fake.calls();
        assert_eq!(
            calls[2].1,
            vec!["stop".to_string(), "kuadrat-pbrain".to_string()]
        );
    }

    #[tokio::test]
    async fn remove_is_ok_when_unit_absent() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());

        remove(&fake, &fs, &paths, "never-existed")
            .await
            .expect("no error");

        // Nothing to stop means nothing was run against the host.
        assert!(fake.calls().is_empty());
    }

    const FOREIGN: &str = "[Container]\nImage=nginx\n";

    /// A fake filesystem pre-loaded with a unit file kuadrat did not write, sitting at
    /// exactly the path kuadrat would use.
    fn seeded_with_foreign_unit(paths: &Paths, name: &str) -> FakeFileSystem {
        let fs = FakeFileSystem::new();
        fs.insert(unit_path(paths, name), FOREIGN);
        fs
    }

    #[tokio::test]
    async fn apply_refuses_to_overwrite_a_foreign_unit_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = seeded_with_foreign_unit(&paths, "nginx");
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("nginx", "alpine");
        let err = apply(&fake, &fs, &paths, &spec)
            .await
            .expect_err("foreign unit is refused");

        assert!(err.to_string().contains("refusing to overwrite"), "{err}");
        // The operator's file is untouched and nothing ran against systemd.
        assert_eq!(
            fs.contents(unit_path(&paths, "nginx")).as_deref(),
            Some(FOREIGN)
        );
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn remove_refuses_to_delete_a_foreign_unit_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = seeded_with_foreign_unit(&paths, "nginx");
        fake.expect("systemctl", ok());

        let err = remove(&fake, &fs, &paths, "nginx")
            .await
            .expect_err("foreign unit is refused");

        assert!(err.to_string().contains("refusing to remove"), "{err}");
        assert!(fs.contents(unit_path(&paths, "nginx")).is_some());
        // Critically: no `systemctl stop nginx` against the host's real nginx.
        assert!(fake.calls().is_empty());
    }

    #[tokio::test]
    async fn apply_never_touches_an_unprefixed_unit_of_the_same_name() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = FakeFileSystem::new();
        fake.expect("systemctl", ok());

        // The operator's hand-written unit, named exactly like the workload.
        let operator_unit = paths.quadlet_dir.join("nginx.container");
        fs.insert(&operator_unit, FOREIGN);

        apply(&fake, &fs, &paths, &WorkloadSpec::new("nginx", "alpine"))
            .await
            .expect("apply succeeds beside the operator's unit");

        assert_eq!(fs.contents(&operator_unit).as_deref(), Some(FOREIGN));
        assert!(fs
            .contents(unit_path(&paths, "nginx"))
            .expect("kuadrat's own unit was written")
            .starts_with(MANAGED_MARKER));
        assert_eq!(
            fake.calls()[1].1,
            vec!["start".to_string(), "kuadrat-nginx".to_string()]
        );
    }
}
