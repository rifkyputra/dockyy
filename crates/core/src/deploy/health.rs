//! The healthcheck stage. A workload with a `health_cmd` is polled via
//! `podman healthcheck run` until healthy or the budget elapses; one without
//! falls back to `systemctl is-active`.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::spec::WorkloadSpec;

const HEALTH_ATTEMPTS: u32 = 30;
const HEALTH_INTERVAL: Duration = Duration::from_secs(2);

/// Wait for a freshly-applied workload to be healthy. Uses the container's
/// podman healthcheck when the spec defines one, else `systemctl is-active`.
pub async fn healthcheck(exec: &dyn Executor, spec: &WorkloadSpec, slug: &str) -> Result<()> {
    let container = format!("kuadrat-{slug}");
    if spec.health_cmd.is_some() {
        poll_health(exec, &container, HEALTH_ATTEMPTS, HEALTH_INTERVAL).await
    } else {
        let out = exec
            .run("systemctl", &["is-active".to_string(), container.clone()])
            .await?;
        if out.stdout.trim() == "active" {
            Ok(())
        } else {
            bail!(
                "{container} is not active after start (is-active: {})",
                out.stdout.trim()
            );
        }
    }
}

/// Poll `podman healthcheck run <container>` until it succeeds or `attempts`
/// checks have failed, sleeping `interval` between checks.
async fn poll_health(
    exec: &dyn Executor,
    container: &str,
    attempts: u32,
    interval: Duration,
) -> Result<()> {
    for attempt in 0..attempts {
        let out = exec
            .run(
                "podman",
                &[
                    "healthcheck".to_string(),
                    "run".to_string(),
                    container.to_string(),
                ],
            )
            .await?;
        if out.success() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(interval).await;
        }
    }
    bail!("{container} did not become healthy after {attempts} checks");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::spec::WorkloadSpec;
    use std::time::Duration;

    fn out(status: i32, stdout: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn healthcheck_polls_podman_when_a_health_cmd_is_set() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.health_cmd = Some("curl -fsS localhost/health".into());
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(0, ""));

        healthcheck(&exec, &spec, "web").await.expect("healthy");
    }

    #[tokio::test]
    async fn poll_health_bails_after_the_attempt_budget() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(1, ""));
        let err = poll_health(&exec, "kuadrat-web", 2, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("healthy"), "message was: {err}");
    }

    #[tokio::test]
    async fn healthcheck_without_a_health_cmd_uses_is_active() {
        let spec = WorkloadSpec::new("worker", "alpine"); // no health_cmd
        let exec = FakeExecutor::new();
        exec.expect_call(
            "systemctl",
            &["is-active", "kuadrat-worker"],
            out(0, "active\n"),
        );

        healthcheck(&exec, &spec, "worker").await.expect("active");
    }

    #[tokio::test]
    async fn healthcheck_without_a_health_cmd_bails_when_inactive() {
        let spec = WorkloadSpec::new("worker", "alpine");
        let exec = FakeExecutor::new();
        exec.expect_call(
            "systemctl",
            &["is-active", "kuadrat-worker"],
            out(3, "failed\n"),
        );

        let err = healthcheck(&exec, &spec, "worker").await.unwrap_err();
        assert!(err.to_string().contains("active"), "message was: {err}");
    }
}
