//! `podman secret` management. Specs carry secret NAMES; values travel only
//! through stdin (via `Executor::run_with_stdin`), never argv, never a log.

use std::collections::HashSet;

use anyhow::{bail, Result};

use crate::exec::Executor;

/// Create or replace a secret. The value is piped to podman; it never appears
/// in argv. Errors name the secret and echo podman's stderr, never the value.
pub async fn set(exec: &dyn Executor, name: &str, value: &str) -> Result<()> {
    // podman 4.9.3's `secret create --replace` is broken for a secret that
    // does not yet exist (it errors trying to delete the nonexistent old
    // one), so upsert as remove-then-create instead of relying on --replace.
    // The remove is best-effort: the secret may not exist yet, so its result
    // is ignored.
    let _ = exec
        .run(
            "podman",
            &["secret".to_string(), "rm".to_string(), name.to_string()],
        )
        .await;
    let out = exec
        .run_with_stdin(
            "podman",
            &[
                "secret".to_string(),
                "create".to_string(),
                name.to_string(),
                "-".to_string(),
            ],
            value,
        )
        .await?;
    if !out.success() {
        bail!(
            "podman secret create failed for {name}: {}",
            out.stderr.trim()
        );
    }
    Ok(())
}

/// Names of every podman secret.
pub async fn list(exec: &dyn Executor) -> Result<Vec<String>> {
    let out = exec
        .run(
            "podman",
            &[
                "secret".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "{{.Name}}".to_string(),
            ],
        )
        .await?;
    if !out.success() {
        bail!("podman secret ls failed: {}", out.stderr.trim());
    }
    Ok(out
        .stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Delete a secret.
pub async fn remove(exec: &dyn Executor, name: &str) -> Result<()> {
    let out = exec
        .run(
            "podman",
            &["secret".to_string(), "rm".to_string(), name.to_string()],
        )
        .await?;
    if !out.success() {
        bail!("podman secret rm failed for {name}: {}", out.stderr.trim());
    }
    Ok(())
}

/// Verify every named secret exists, bailing with the missing names. The
/// Secrets stage's pre-flight: a missing credential must fail before Apply.
pub async fn ensure_all(exec: &dyn Executor, names: &[String]) -> Result<()> {
    let have: HashSet<String> = list(exec).await?.into_iter().collect();
    let missing: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| !have.contains(*n))
        .collect();
    if !missing.is_empty() {
        bail!("missing secrets: {}", missing.join(", "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn set_passes_the_value_by_stdin_never_argv() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["secret", "rm", "db-pw"], ok(""));
        exec.expect_call("podman", &["secret", "create", "db-pw", "-"], ok(""));

        set(&exec, "db-pw", "supersecret").await.expect("set");

        // Value went through stdin...
        assert_eq!(exec.stdins(), vec!["supersecret".to_string()]);
        // ...and never into the argv log.
        let flat = format!("{:?}", exec.calls());
        assert!(
            !flat.contains("supersecret"),
            "argv leaked the secret: {flat}"
        );
    }

    #[tokio::test]
    async fn set_creates_when_the_secret_is_new() {
        let exec = FakeExecutor::new();
        // The secret does not exist yet, so the best-effort rm fails — set must ignore that.
        exec.expect_call(
            "podman",
            &["secret", "rm", "fresh"],
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "no such secret".into(),
            },
        );
        exec.expect_call("podman", &["secret", "create", "fresh", "-"], ok(""));

        set(&exec, "fresh", "v").await.expect("set on a new name");
        assert_eq!(exec.stdins(), vec!["v".to_string()]);
    }

    #[tokio::test]
    async fn set_fails_without_echoing_the_value() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["secret", "rm", "db-pw"], ok(""));
        exec.expect_call(
            "podman",
            &["secret", "create", "db-pw", "-"],
            CommandOutput {
                status: 125,
                stdout: String::new(),
                stderr: "disk full".into(),
            },
        );
        let err = set(&exec, "db-pw", "supersecret").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("db-pw"), "message was: {msg}");
        assert!(
            !msg.contains("supersecret"),
            "error leaked the value: {msg}"
        );
    }

    #[tokio::test]
    async fn list_parses_names() {
        let exec = FakeExecutor::new();
        exec.expect_call(
            "podman",
            &["secret", "ls", "--format", "{{.Name}}"],
            ok("alpha\nbeta\n"),
        );
        let names = list(&exec).await.expect("list");
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[tokio::test]
    async fn remove_calls_podman_secret_rm() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["secret", "rm", "db-pw"], ok(""));
        remove(&exec, "db-pw").await.expect("remove");
    }

    #[tokio::test]
    async fn ensure_all_passes_when_present_and_names_the_missing() {
        let exec = FakeExecutor::new();
        exec.expect("podman", ok("alpha\nbeta\n"));

        ensure_all(&exec, &["alpha".to_string()])
            .await
            .expect("present");

        let err = ensure_all(&exec, &["alpha".to_string(), "gamma".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("gamma"), "message was: {err}");
    }
}
