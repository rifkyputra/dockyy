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

    /// Run a command, feeding `stdin` to it. Used for secret values, which must
    /// never appear in argv (world-readable via `ps`). Default impl bails so a
    /// new executor compiles until it opts in.
    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[String],
        stdin: &str,
    ) -> Result<CommandOutput> {
        let _ = (program, args, stdin);
        anyhow::bail!("run_with_stdin is not supported by this executor")
    }
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

    fn out(status: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            status,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    /// The gap this closes: with per-program scripting only, every `systemctl`
    /// call in a test returns the same output, so "one subcommand succeeds and
    /// the next fails" cannot be expressed at all.
    #[tokio::test]
    async fn fake_executor_scripts_per_argv() {
        let fake = FakeExecutor::new();
        fake.expect_call("systemctl", &["daemon-reload"], out(0, ""));
        fake.expect_call("systemctl", &["start", "web"], out(1, "start refused"));

        let reload = fake
            .run("systemctl", &["daemon-reload".to_string()])
            .await
            .expect("scripted");
        assert!(reload.success());

        let start = fake
            .run("systemctl", &["start".to_string(), "web".to_string()])
            .await
            .expect("scripted");
        assert!(!start.success());
        assert_eq!(start.stderr, "start refused");
    }

    #[tokio::test]
    async fn exact_argv_wins_over_the_program_fallback() {
        let fake = FakeExecutor::new();
        fake.expect("systemctl", out(0, ""));
        fake.expect_call("systemctl", &["start", "web"], out(1, "nope"));

        let stop = fake
            .run("systemctl", &["stop".to_string(), "web".to_string()])
            .await
            .expect("falls back");
        assert!(stop.success());

        let start = fake
            .run("systemctl", &["start".to_string(), "web".to_string()])
            .await
            .expect("exact match");
        assert_eq!(start.stderr, "nope");
    }

    #[tokio::test]
    async fn unexpected_call_error_names_the_argv() {
        let fake = FakeExecutor::new();
        fake.expect_call("systemctl", &["daemon-reload"], out(0, ""));

        let err = fake
            .run("systemctl", &["start".to_string(), "web".to_string()])
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("systemctl"), "message was: {msg}");
        assert!(msg.contains("start"), "message was: {msg}");
        assert!(msg.contains("web"), "message was: {msg}");
    }

    fn ok_out(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn fake_run_with_stdin_records_argv_but_never_the_stdin() {
        let fake = FakeExecutor::new();
        fake.expect_call("podman", &["secret", "create", "db"], ok_out(""));

        let out = fake
            .run_with_stdin(
                "podman",
                &["secret".to_string(), "create".to_string(), "db".to_string()],
                "the-secret-value",
            )
            .await
            .expect("scripted");
        assert!(out.success());

        // The argv is recorded...
        let calls = fake.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "podman");
        // ...but the secret value is nowhere in the call log.
        let flat = format!("{:?}", calls);
        assert!(
            !flat.contains("the-secret-value"),
            "call log leaked the secret: {flat}"
        );

        // The dedicated accessor exposes it for the one test that needs it.
        assert_eq!(fake.stdins(), vec!["the-secret-value".to_string()]);
    }

    #[tokio::test]
    async fn default_run_with_stdin_bails() {
        struct Bare;
        #[async_trait::async_trait]
        impl Executor for Bare {
            async fn run(&self, _program: &str, _args: &[String]) -> anyhow::Result<CommandOutput> {
                Ok(CommandOutput {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }
        }
        let err = Bare.run_with_stdin("x", &[], "y").await.unwrap_err();
        assert!(err.to_string().contains("stdin"), "message was: {err}");
    }

    #[tokio::test]
    async fn local_run_with_stdin_feeds_the_child() {
        let exec = LocalExecutor;
        // `cat` echoes stdin back on stdout.
        let out = exec
            .run_with_stdin("cat", &[], "piped-input")
            .await
            .expect("cat runs");
        assert!(out.success());
        assert_eq!(out.stdout, "piped-input");
    }
}
