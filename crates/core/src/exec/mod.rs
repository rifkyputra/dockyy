pub mod fake;
pub mod local;

use anyhow::Result;
use async_trait::async_trait;

/// Result of running one host command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == 0
    }
}

/// The single seam through which `core` touches the host.
///
/// Every `podman` and `systemctl` invocation goes through this. Implementations:
/// `LocalExecutor` (shells out), `FakeExecutor` (tests), and later a remote executor
/// over SSH for the fleet driver.
#[async_trait]
pub trait Executor: Send + Sync {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput>;
}

#[cfg(test)]
mod tests {
    use super::{CommandOutput, Executor};
    use crate::exec::fake::FakeExecutor;
    use crate::exec::local::LocalExecutor;

    #[test]
    fn command_output_success_reflects_status() {
        let ok = CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        };
        let bad = CommandOutput {
            status: 1,
            stdout: String::new(),
            stderr: String::new(),
        };
        assert!(ok.success());
        assert!(!bad.success());
    }

    #[tokio::test]
    async fn local_executor_runs_a_real_command() {
        let exec = LocalExecutor;
        let out = exec
            .run("echo", &["hello".to_string()])
            .await
            .expect("echo runs");
        assert!(out.success());
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[tokio::test]
    async fn fake_executor_returns_scripted_output_and_records_calls() {
        let fake = FakeExecutor::new();
        fake.expect(
            "podman",
            CommandOutput {
                status: 0,
                stdout: "ok".into(),
                stderr: String::new(),
            },
        );

        let out = fake
            .run("podman", &["ps".to_string()])
            .await
            .expect("scripted");
        assert_eq!(out.stdout, "ok");

        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "podman");
        assert_eq!(calls[0].1, vec!["ps".to_string()]);
    }

    #[tokio::test]
    async fn fake_executor_errors_on_unexpected_program() {
        let fake = FakeExecutor::new();
        let err = fake.run("systemctl", &[]).await.unwrap_err();
        assert!(err.to_string().contains("systemctl"));
    }
}
