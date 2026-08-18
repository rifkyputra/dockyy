use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::managed::ensure_owned;
use crate::spec::WorkloadSpec;
use crate::workloads::paths::{task_container_path, task_stem_prefix, task_timer_path};
use crate::workloads::render::{render, render_task, render_timer, MANAGED_MARKER};

pub use crate::workloads::paths::{task_unit_name, unit_name, unit_path, Paths};

/// Write the unit, reload systemd, and start the workload — and its scheduled
/// tasks: one oneshot `.container` + `.timer` per task, enabled after the
/// reload, with task units the spec no longer names pruned.
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

    // Preflight every schedule before ANY write. `systemctl enable` accepts a
    // timer whose OnCalendar cannot parse — the timer just never fires,
    // silently. This turns that into an error naming the task, now.
    for task in &spec.tasks {
        let out = exec
            .run(
                "systemd-analyze",
                &["calendar".to_string(), task.schedule.clone()],
            )
            .await?;
        if !out.success() {
            bail!(
                "task {:?} has an invalid schedule: {}",
                task.name,
                out.stderr.trim()
            );
        }
    }

    // Render everything before writing anything.
    struct TaskFiles {
        container: PathBuf,
        container_text: String,
        timer: PathBuf,
        timer_text: String,
        timer_unit: String,
    }
    let mut task_files = Vec::with_capacity(spec.tasks.len());
    for task in &spec.tasks {
        task_files.push(TaskFiles {
            container: task_container_path(paths, &spec.name, &task.name),
            container_text: render_task(spec, task)?,
            timer: task_timer_path(paths, &spec.name, &task.name),
            timer_text: render_timer(spec, task)?,
            timer_unit: format!("{}.timer", task_unit_name(&spec.name, &task.name)),
        });
    }

    let path = unit_path(paths, &spec.name);
    ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite").await?;
    for tf in &task_files {
        ensure_owned(fsys, &tf.container, MANAGED_MARKER, "overwrite").await?;
        ensure_owned(fsys, &tf.timer, MANAGED_MARKER, "overwrite").await?;
    }

    let previous = if fsys.exists(&path).await? {
        Some(fsys.read_to_string(&path).await?)
    } else {
        None
    };

    fsys.create_dir_all(&paths.quadlet_dir).await?;
    if !task_files.is_empty() {
        fsys.create_dir_all(&paths.systemd_dir).await?;
    }
    fsys.write(&path, &unit).await?;
    for tf in &task_files {
        fsys.write(&tf.container, &tf.container_text).await?;
        fsys.write(&tf.timer, &tf.timer_text).await?;
    }

    // Prune task units this spec no longer names — disable their timers while
    // systemd still knows them, then delete the files.
    let keep: HashSet<String> = spec
        .tasks
        .iter()
        .map(|t| task_unit_name(&spec.name, &t.name))
        .collect();
    prune_tasks(exec, fsys, paths, &spec.name, &keep).await?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;

    // A new or byte-identical unit only needs `start` (a no-op if already
    // running). A changed unit needs `restart`, or the old container keeps
    // running behind the new unit file.
    let changed = matches!(&previous, Some(p) if p != &unit);
    let action = if changed { "restart" } else { "start" };
    systemctl(exec, &[action.to_string(), unit_name(&spec.name)]).await?;

    for tf in &task_files {
        systemctl(
            exec,
            &[
                "enable".to_string(),
                "--now".to_string(),
                tf.timer_unit.clone(),
            ],
        )
        .await?;
    }

    Ok(())
}

/// Stop the workload, delete its unit and its tasks' units, and reload
/// systemd. Safe if absent.
///
/// Refuses to delete a unit file kuadrat does not own. Tasks are cleaned even
/// when the main unit is already gone, so an orphaned timer cannot outlive
/// its app.
pub async fn remove(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    name: &str,
) -> Result<()> {
    let pruned = prune_tasks(exec, fsys, paths, name, &HashSet::new()).await?;

    let path = unit_path(paths, name);
    if !ensure_owned(fsys, &path, MANAGED_MARKER, "remove").await? {
        if pruned {
            systemctl(exec, &["daemon-reload".to_string()]).await?;
        }
        return Ok(());
    }

    systemctl(exec, &["stop".to_string(), unit_name(name)]).await?;
    fsys.remove_file(&path).await?;
    systemctl(exec, &["daemon-reload".to_string()]).await?;

    Ok(())
}

/// Delete this app's task files whose unit stem is not in `keep`, disabling
/// each stale `.timer` first (while systemd still knows it). Ownership-gated:
/// a matching filename without the managed marker is refused, never deleted.
/// Returns whether anything was removed.
async fn prune_tasks(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    spec_name: &str,
    keep: &HashSet<String>,
) -> Result<bool> {
    let prefix = task_stem_prefix(spec_name);
    let mut removed = false;
    for dir in [&paths.quadlet_dir, &paths.systemd_dir] {
        if !fsys.exists(dir).await? {
            continue;
        }
        for entry in fsys.read_dir(dir).await? {
            let Some(stem) = entry
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if !stem.starts_with(&prefix) || keep.contains(&stem) {
                continue;
            }
            if !ensure_owned(fsys, &entry, MANAGED_MARKER, "remove").await? {
                continue;
            }
            if entry.extension().is_some_and(|e| e == "timer") {
                systemctl(
                    exec,
                    &[
                        "disable".to_string(),
                        "--now".to_string(),
                        format!("{stem}.timer"),
                    ],
                )
                .await?;
            }
            fsys.remove_file(&entry).await?;
            removed = true;
        }
    }
    Ok(removed)
}

async fn systemctl(exec: &dyn Executor, args: &[String]) -> Result<()> {
    let out = exec.run("systemctl", args).await?;
    if !out.success() {
        bail!("systemctl {} failed: {}", args.join(" "), out.stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests_tasks {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use crate::fs::local::LocalFileSystem;
    use crate::spec::{ScheduledTask, WorkloadSpec};
    use crate::workloads::paths::{task_container_path, task_timer_path};
    use tempfile::tempdir;

    fn ok() -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn spec_with_cleanup_task() -> WorkloadSpec {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.tasks = vec![ScheduledTask {
            name: "cleanup".into(),
            schedule: "daily".into(),
            command: vec!["true".into()],
        }];
        spec
    }

    #[tokio::test]
    async fn apply_writes_task_units_and_enables_their_timers() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());
        fake.expect("systemd-analyze", ok());

        apply(&fake, &fs, &paths, &spec_with_cleanup_task())
            .await
            .expect("apply");

        assert!(task_container_path(&paths, "web", "cleanup").exists());
        assert!(task_timer_path(&paths, "web", "cleanup").exists());

        let calls = fake.calls();
        assert!(
            calls
                .iter()
                .any(|(p, a)| p == "systemd-analyze" && a == &vec!["calendar", "daily"]),
            "no schedule preflight: {calls:?}"
        );
        assert!(
            calls.iter().any(|(_, a)| a
                == &vec![
                    "enable".to_string(),
                    "--now".to_string(),
                    "kuadrat-web-task-cleanup.timer".to_string()
                ]),
            "timer not enabled: {calls:?}"
        );
    }

    #[tokio::test]
    async fn an_invalid_schedule_fails_before_any_file_is_written() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect(
            "systemd-analyze",
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "Failed to parse calendar specification".into(),
            },
        );

        let err = apply(&fake, &fs, &paths, &spec_with_cleanup_task())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("cleanup"), "{err}");
        assert!(!unit_path(&paths, "web").exists(), "main unit was written");
        assert!(!task_container_path(&paths, "web", "cleanup").exists());
        // Only the preflight ran; systemd was never touched.
        assert!(fake.calls().iter().all(|(p, _)| p == "systemd-analyze"));
    }

    #[tokio::test]
    async fn a_task_removed_from_the_spec_is_pruned_on_apply() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());
        fake.expect("systemd-analyze", ok());

        apply(&fake, &fs, &paths, &spec_with_cleanup_task())
            .await
            .expect("first apply");
        // Same app, no tasks: the cleanup task's units must go.
        apply(&fake, &fs, &paths, &WorkloadSpec::new("web", "alpine"))
            .await
            .expect("second apply");

        assert!(!task_container_path(&paths, "web", "cleanup").exists());
        assert!(!task_timer_path(&paths, "web", "cleanup").exists());
        let calls = fake.calls();
        assert!(
            calls.iter().any(|(_, a)| a
                == &vec![
                    "disable".to_string(),
                    "--now".to_string(),
                    "kuadrat-web-task-cleanup.timer".to_string()
                ]),
            "stale timer not disabled: {calls:?}"
        );
    }

    #[tokio::test]
    async fn a_foreign_file_matching_the_task_prefix_is_refused_not_deleted() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = FakeFileSystem::new();
        fake.expect("systemctl", ok());
        fake.expect("systemd-analyze", ok());

        // An operator's own file at exactly the path kuadrat would use.
        let foreign = "[Container]\nImage=evil\n";
        fs.insert(task_container_path(&paths, "web", "cleanup"), foreign);

        let err = apply(&fake, &fs, &paths, &spec_with_cleanup_task())
            .await
            .expect_err("foreign task file is refused");

        assert!(err.to_string().contains("refusing to overwrite"), "{err}");
        assert_eq!(
            fs.contents(task_container_path(&paths, "web", "cleanup"))
                .as_deref(),
            Some(foreign)
        );
    }

    #[tokio::test]
    async fn remove_cleans_up_task_units_with_the_app() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        let fs = LocalFileSystem;
        fake.expect("systemctl", ok());
        fake.expect("systemd-analyze", ok());

        apply(&fake, &fs, &paths, &spec_with_cleanup_task())
            .await
            .expect("apply");
        remove(&fake, &fs, &paths, "web").await.expect("remove");

        assert!(!task_container_path(&paths, "web", "cleanup").exists());
        assert!(!task_timer_path(&paths, "web", "cleanup").exists());
        assert!(!unit_path(&paths, "web").exists());
        let calls = fake.calls();
        assert!(
            calls.iter().any(|(_, a)| a
                == &vec![
                    "disable".to_string(),
                    "--now".to_string(),
                    "kuadrat-web-task-cleanup.timer".to_string()
                ]),
            "timer not disabled on remove: {calls:?}"
        );
    }
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

    /// Previously inexpressible: with per-program scripting, `daemon-reload` and
    /// `start` shared one scripted result, so a test could not make the first
    /// succeed and the second fail. This is the shape phase 2's per-stage
    /// compensation tests are built on.
    #[tokio::test]
    async fn apply_fails_at_start_after_a_successful_reload() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;
        let fake = FakeExecutor::new();
        fake.expect_call("systemctl", &["daemon-reload"], ok());
        fake.expect_call(
            "systemctl",
            &["start", "kuadrat-pbrain"],
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "job failed".into(),
            },
        );

        let spec = WorkloadSpec::new("pbrain", "alpine");
        let err = apply(&fake, &fs, &paths, &spec).await.unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("start"), "message was: {msg}");
        assert!(msg.contains("job failed"), "message was: {msg}");

        // The reload ran and succeeded; only the start failed.
        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, vec!["daemon-reload".to_string()]);

        // Phase-2 note: the unit file is left on disk. Compensation is the
        // deploy state machine's job, not apply's — see docs/known-gaps.md.
        assert!(unit_path(&paths, "pbrain").exists());
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
    async fn redeploying_a_changed_spec_restarts_not_starts() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        // First apply: new unit → start.
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("first apply");

        // Second apply: different image → the unit content changes → restart.
        spec.image = "alpine:3.20".to_string();
        apply(&fake, &fs, &paths, &spec)
            .await
            .expect("second apply");

        let calls = fake.calls();
        // The final systemctl action must be a restart of the changed unit.
        let restarted = calls
            .iter()
            .any(|(_, a)| a == &vec!["restart".to_string(), "kuadrat-pbrain".to_string()]);
        assert!(
            restarted,
            "expected a restart of the changed unit; calls: {calls:?}"
        );
    }

    #[tokio::test]
    async fn reapplying_an_unchanged_spec_starts_not_restarts() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        // First apply: new unit → start.
        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("first apply");

        // Second apply: identical spec → byte-identical unit → start, not restart.
        apply(&fake, &fs, &paths, &spec)
            .await
            .expect("second apply");

        let calls = fake.calls();
        let restarted = calls
            .iter()
            .any(|(_, a)| a == &vec!["restart".to_string(), "kuadrat-pbrain".to_string()]);
        assert!(
            !restarted,
            "unchanged reapply must not restart; calls: {calls:?}"
        );
        assert_eq!(
            calls[3].1,
            vec!["start".to_string(), "kuadrat-pbrain".to_string()],
            "second apply's systemctl action must be start; calls: {calls:?}"
        );
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
