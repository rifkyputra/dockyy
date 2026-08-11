pub mod fake;
pub mod local;

use anyhow::Result;
use async_trait::async_trait;
use tokio_stream::Stream;

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

    /// Run a command and yield its stdout a line at a time, for as long as it
    /// runs.
    ///
    /// Returns a stream rather than taking a channel because a channel-based
    /// signature does not return until the stream ends, so a caller cannot
    /// both drive it and read it in one task — it would have to spawn, and
    /// `spawn` needs `'static` while `core` holds `&dyn Executor` everywhere.
    /// A seam that dictates its caller's task structure has stopped being an
    /// abstraction. The `Result` per item puts a mid-stream failure inline,
    /// where it happened, instead of on a separate path from the lines it
    /// interrupted.
    ///
    /// Default impl bails, like `run_with_stdin`, so a new executor compiles
    /// until it opts in.
    async fn run_streaming(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>> {
        let _ = (program, args);
        anyhow::bail!("streaming is not supported by this executor")
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

    /// Regression test for the orphan-process bug: callers now wrap `exec.run`
    /// in `tokio::time::timeout` (the healthcheck stage) and drop the future
    /// when the deadline fires. Without `kill_on_drop(true)` on the underlying
    /// `Command`, the spawned child keeps running after its future is gone.
    ///
    /// The child writes its own PID to a file, then sleeps far longer than the
    /// timeout below. We drop the `run` future via `timeout`, then poll `/proc`
    /// for that PID to confirm the process actually exits. The 2s poll budget
    /// is generous relative to the 50ms timeout, so this should not be flaky
    /// on any machine that can run a shell at all.
    #[tokio::test]
    async fn local_executor_kills_child_when_future_is_dropped() {
        let dir = std::env::temp_dir();
        let pidfile = dir.join(format!("kuadrat-kill-on-drop-test-{}", std::process::id()));
        let pidfile_str = pidfile.to_str().expect("utf8 path").to_string();
        let _ = std::fs::remove_file(&pidfile);

        let exec = LocalExecutor;
        let script = format!("echo $$ > {pidfile_str}; sleep 30");
        let args = ["-c".to_string(), script];
        let fut = exec.run("sh", &args);

        let result = tokio::time::timeout(std::time::Duration::from_millis(50), fut).await;
        assert!(
            result.is_err(),
            "the command should still be running at 50ms"
        );
        // `result` (and the dropped `fut` inside it) is gone now; `kill_on_drop`
        // should have signaled the child as part of that drop.

        // Wait for the PID file to appear, then confirm the process it names
        // is gone. Poll rather than sleep-and-check-once so this isn't tied to
        // one guessed timing.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let pid = loop {
            if let Ok(contents) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = contents.trim().parse::<u32>() {
                    break pid;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child never wrote its pid file"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };

        loop {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "child pid {pid} is still alive; kill_on_drop did not kill it"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let _ = std::fs::remove_file(&pidfile);
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

    /// A new executor — the fleet driver's SSH one, later — must compile
    /// before it supports streaming. The default impl is what allows that,
    /// and it must fail loudly rather than silently yielding nothing.
    #[tokio::test]
    async fn an_executor_that_has_not_opted_in_bails_on_run_streaming() {
        struct Minimal;
        #[async_trait::async_trait]
        impl Executor for Minimal {
            async fn run(&self, _p: &str, _a: &[String]) -> anyhow::Result<CommandOutput> {
                unreachable!()
            }
        }
        // `Result::unwrap_err` requires the `Ok` type to be `Debug`, which a
        // boxed `dyn Stream` trait object cannot be (a second, non-auto trait
        // bound isn't allowed on a trait object). Match instead.
        let err = match Minimal.run_streaming("journalctl", &[]).await {
            Ok(_) => panic!("expected the default impl to bail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("streaming"), "was: {err}");
    }

    #[tokio::test]
    async fn the_local_executor_yields_stdout_a_line_at_a_time() {
        use tokio_stream::StreamExt;
        let exec = LocalExecutor;
        let mut stream = exec
            .run_streaming("sh", &["-c".into(), "printf 'one\\ntwo\\nthree\\n'".into()])
            .await
            .expect("stream");

        let mut got = Vec::new();
        while let Some(line) = stream.next().await {
            got.push(line.expect("line"));
        }
        assert_eq!(got, vec!["one", "two", "three"]);
    }

    /// Dropping the stream must kill the child. Without that, "the client went
    /// away" is an intention rather than a bound, and every abandoned viewer
    /// leaves a `journalctl -f` running.
    #[tokio::test]
    async fn dropping_the_stream_kills_the_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pidfile = dir.path().join("pid");
        let script = format!("echo $$ > {}; sleep 30", pidfile.display());

        let exec = LocalExecutor;
        let stream = exec
            .run_streaming("sh", &["-c".into(), script])
            .await
            .expect("stream");

        // Wait for the child to record its pid rather than sleeping a fixed
        // amount: a sleep is slow when it passes and flaky when it does not.
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        };

        drop(stream);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "child {pid} outlived its stream"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn the_fake_executor_yields_its_scripted_lines() {
        use tokio_stream::StreamExt;
        let exec = FakeExecutor::new();
        exec.expect_stream("journalctl", vec!["a".into(), "b".into()]);

        let mut stream = exec.run_streaming("journalctl", &[]).await.expect("stream");
        let mut got = Vec::new();
        while let Some(line) = stream.next().await {
            got.push(line.expect("line"));
        }
        assert_eq!(got, vec!["a", "b"]);
    }
}
