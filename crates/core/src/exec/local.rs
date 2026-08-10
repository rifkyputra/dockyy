use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{CommandOutput, Executor};

/// Runs commands on the local host.
pub struct LocalExecutor;

#[async_trait]
impl Executor for LocalExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        let output = Command::new(program)
            .args(args)
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

        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
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
}
