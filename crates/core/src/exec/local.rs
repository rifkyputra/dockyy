use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio_stream::wrappers::LinesStream;
use tokio_stream::Stream;

use super::{CommandOutput, Executor};

/// Runs commands on the local host.
pub struct LocalExecutor;

#[async_trait]
impl Executor for LocalExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        // `kill_on_drop(true)`: callers now wrap this future in `tokio::time::timeout`
        // (e.g. the healthcheck stage) and drop it when the deadline fires. Without
        // this, the spawned child keeps running after its future is gone — an orphan
        // process that is invisible to everything kuadrat can see.
        let output = Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("failed to run `{program}`"))?;

        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[String],
        stdin: &str,
    ) -> Result<CommandOutput> {
        use std::process::Stdio;

        // See `run` above: `kill_on_drop(true)` ensures a dropped future (e.g. on
        // timeout) does not leave this child running as an untracked orphan.
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn `{program}`"))?;

        child
            .stdin
            .take()
            .context("child stdin was not piped")?
            .write_all(stdin.as_bytes())
            .await
            .context("writing to child stdin")?;
        // The taken stdin drops here, closing the pipe so the child sees EOF.

        let output = child
            .wait_with_output()
            .await
            .with_context(|| format!("failed to run `{program}`"))?;

        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn run_streaming(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<Box<dyn Stream<Item = Result<String>> + Send + Unpin>> {
        use std::process::Stdio;

        // See `run` above: `kill_on_drop(true)` ensures a dropped/abandoned
        // stream does not leave this child running as an untracked orphan.
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn `{program}`"))?;

        let stdout = child.stdout.take().context("child stdout was not piped")?;
        let lines = LinesStream::new(BufReader::new(stdout).lines());

        Ok(Box::new(ChildLines {
            _child: child,
            lines,
        }))
    }
}

/// A child's stdout, line by line, with the child kept alive alongside it.
///
/// The `Child` is held here rather than detached so that dropping the stream
/// drops the child — and `kill_on_drop(true)` then kills it. That is what makes
/// "the consumer went away" an enforced bound rather than an intention.
struct ChildLines {
    _child: Child,
    lines: LinesStream<BufReader<ChildStdout>>,
}

impl Stream for ChildLines {
    type Item = Result<String>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.lines)
            .poll_next(cx)
            .map(|item| item.map(|r| r.map_err(anyhow::Error::from)))
    }
}
