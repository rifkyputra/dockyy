//! The healthcheck stage. A workload with a `health_cmd` is polled via
//! `podman healthcheck run` until healthy or a wall-clock budget elapses; one
//! without falls back to `systemctl is-active`.
//!
//! The budget is expressed in time, not attempt count: `LocalExecutor::run`
//! has no timeout of its own, so an attempt count alone puts no bound on how
//! long the stage can run if a single health command hangs. Each attempt is
//! itself wrapped in [`HEALTH_ATTEMPT_TIMEOUT`] so one hang can only ever cost
//! that much, not the whole budget.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::spec::WorkloadSpec;
use crate::workloads::render::HEALTH_ATTEMPT_TIMEOUT;

/// Wall-clock ceiling for the whole healthcheck stage — the same nominal
/// ceiling the old 30 attempts x 2s interval implied.
const HEALTH_BUDGET: Duration = Duration::from_secs(60);
const HEALTH_INTERVAL: Duration = Duration::from_secs(2);

/// Wait for a freshly-applied workload to be healthy. Uses the container's
/// podman healthcheck when the spec defines one, else `systemctl is-active`.
pub async fn healthcheck(exec: &dyn Executor, spec: &WorkloadSpec) -> Result<()> {
    let slug = spec.slug();
    let container = format!("kuadrat-{slug}");
    if spec.health_cmd.is_some() {
        poll_health(exec, &container, HEALTH_BUDGET, HEALTH_INTERVAL).await
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

/// Poll `podman healthcheck run <container>` until it succeeds or `budget`
/// has elapsed, sleeping `interval` between checks. Each individual attempt
/// is capped at [`HEALTH_ATTEMPT_TIMEOUT`]; a timed-out attempt counts as a
/// failed attempt, not an error that ends the stage, because a health command
/// that hangs once is exactly the case the retry loop exists for.
async fn poll_health(
    exec: &dyn Executor,
    container: &str,
    budget: Duration,
    interval: Duration,
) -> Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        let attempt = tokio::time::timeout(
            HEALTH_ATTEMPT_TIMEOUT,
            exec.run(
                "podman",
                &[
                    "healthcheck".to_string(),
                    "run".to_string(),
                    container.to_string(),
                ],
            ),
        )
        .await;
        let healthy = match attempt {
            Ok(Ok(out)) => out.success(),
            Ok(Err(err)) => return Err(err),
            Err(_elapsed) => false,
        };
        if healthy {
            return Ok(());
        }
        if start.elapsed() >= budget {
            break;
        }
        tokio::time::sleep(interval).await;
        if start.elapsed() >= budget {
            break;
        }
    }
    bail!(
        "{container} did not become healthy within {}s",
        budget.as_secs()
    );
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

        healthcheck(&exec, &spec).await.expect("healthy");
    }

    #[tokio::test(start_paused = true)]
    async fn poll_health_bails_after_the_wall_clock_budget() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(1, ""));
        let err = poll_health(
            &exec,
            "kuadrat-web",
            Duration::from_secs(6),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("within 6s"),
            "message names the budget in seconds, was: {err}"
        );
    }

    /// A health command that never succeeds must end the stage within its
    /// budget rather than running forever — the defect this whole change
    /// fixes. Paused clock keeps the test's real wall time near zero even
    /// though virtual time advances past the budget.
    #[tokio::test(start_paused = true)]
    async fn poll_health_never_healthy_ends_within_the_budget() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(1, ""));

        let started = tokio::time::Instant::now();
        let err = poll_health(
            &exec,
            "kuadrat-web",
            Duration::from_secs(10),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("within 10s"), "message: {err}");
        assert!(
            started.elapsed() <= Duration::from_secs(10),
            "poll ran past its budget: {:?}",
            started.elapsed()
        );
    }

    /// A single attempt that hangs must not consume the whole budget: the
    /// per-attempt timeout bounds it, and polling continues afterwards. Uses
    /// a hand-rolled executor since `FakeExecutor` returns synchronously and
    /// cannot simulate a slow podman call.
    struct SlowThenHealthyExecutor {
        first_call_delay: Duration,
        calls: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl Executor for SlowThenHealthyExecutor {
        async fn run(
            &self,
            _program: &str,
            _args: &[String],
        ) -> anyhow::Result<crate::exec::CommandOutput> {
            let call_no = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if call_no == 1 {
                tokio::time::sleep(self.first_call_delay).await;
                Ok(out(1, ""))
            } else {
                Ok(out(0, ""))
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_single_hanging_attempt_does_not_consume_the_whole_budget() {
        let exec = SlowThenHealthyExecutor {
            // Longer than HEALTH_ATTEMPT_TIMEOUT (5s), so the first attempt
            // must be cut off by the per-attempt timeout rather than by
            // actually finishing.
            first_call_delay: Duration::from_secs(20),
            calls: std::sync::atomic::AtomicU32::new(0),
        };

        poll_health(
            &exec,
            "kuadrat-web",
            Duration::from_secs(30),
            Duration::from_secs(1),
        )
        .await
        .expect("second attempt reports healthy");

        assert!(
            exec.calls.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "expected the poll to retry after the hung attempt timed out"
        );
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

        healthcheck(&exec, &spec).await.expect("active");
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

        let err = healthcheck(&exec, &spec).await.unwrap_err();
        assert!(err.to_string().contains("active"), "message was: {err}");
    }
}
