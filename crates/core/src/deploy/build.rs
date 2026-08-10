use anyhow::{bail, Result};

use crate::deploy::detect::BuildPlan;
use crate::exec::Executor;

/// The image reference kuadrat builds for an app at a commit. The `localhost/`
/// prefix marks it local-only, so Quadlet's `Image=` never attempts a pull.
pub fn image_reference(slug: &str, commit: &str) -> String {
    format!("localhost/kuadrat-{slug}:{commit}")
}

/// Build the image with `podman build`, tagged with the app's commit. Returns
/// the image reference on success.
pub async fn build(exec: &dyn Executor, plan: &BuildPlan, slug: &str) -> Result<String> {
    let image = image_reference(slug, &plan.commit);
    let out = exec
        .run(
            "podman",
            &[
                "build".to_string(),
                "-t".to_string(),
                image.clone(),
                "-f".to_string(),
                plan.containerfile.to_string_lossy().into_owned(),
                plan.context_dir.to_string_lossy().into_owned(),
            ],
        )
        .await?;
    if !out.success() {
        bail!("podman build failed: {}", out.stderr.trim());
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::detect::BuildPlan;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use std::path::PathBuf;

    fn plan() -> BuildPlan {
        BuildPlan {
            containerfile: PathBuf::from("/repo/Containerfile"),
            context_dir: PathBuf::from("/repo"),
            commit: "abc123".to_string(),
        }
    }

    fn ok() -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    #[test]
    fn image_reference_is_namespaced_and_local() {
        assert_eq!(
            image_reference("web", "abc123"),
            "localhost/kuadrat-web:abc123"
        );
    }

    #[tokio::test]
    async fn build_invokes_podman_and_returns_the_reference() {
        let exec = FakeExecutor::new();
        exec.expect_call(
            "podman",
            &[
                "build",
                "-t",
                "localhost/kuadrat-web:abc123",
                "-f",
                "/repo/Containerfile",
                "/repo",
            ],
            ok(),
        );

        let image = build(&exec, &plan(), "web").await.expect("build");
        assert_eq!(image, "localhost/kuadrat-web:abc123");
    }

    #[tokio::test]
    async fn build_fails_when_podman_fails() {
        let exec = FakeExecutor::new();
        exec.expect_call(
            "podman",
            &[
                "build",
                "-t",
                "localhost/kuadrat-web:abc123",
                "-f",
                "/repo/Containerfile",
                "/repo",
            ],
            CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "build step failed".into(),
            },
        );

        let err = build(&exec, &plan(), "web").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("build"), "message was: {msg}");
        assert!(msg.contains("build step failed"), "message was: {msg}");
    }
}
