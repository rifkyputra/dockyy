use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::fs::FileSystem;

/// What `build` needs to produce an image: the Containerfile, the build
/// context, and the git commit that becomes the image tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub containerfile: PathBuf,
    pub context_dir: PathBuf,
    pub commit: String,
}

/// Inspect a local repo: find its Containerfile (or Dockerfile) and read its
/// HEAD commit. Fails if neither file exists or the path is not a git repo.
///
/// Reads the git ref only — never fetches. The operator or CI puts the code on
/// disk; kuadrat builds what is there.
pub async fn detect(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    context_dir: &Path,
) -> Result<BuildPlan> {
    let containerfile = {
        let cf = context_dir.join("Containerfile");
        let df = context_dir.join("Dockerfile");
        if fsys.exists(&cf).await? {
            cf
        } else if fsys.exists(&df).await? {
            df
        } else {
            bail!(
                "no Containerfile or Dockerfile in {}",
                context_dir.display()
            );
        }
    };

    let dir = context_dir.to_string_lossy().into_owned();
    let out = exec
        .run(
            "git",
            &[
                "-C".to_string(),
                dir,
                "rev-parse".to_string(),
                "HEAD".to_string(),
            ],
        )
        .await?;
    if !out.success() {
        bail!(
            "{} is not a git repository: {}",
            context_dir.display(),
            out.stderr.trim()
        );
    }

    Ok(BuildPlan {
        containerfile,
        context_dir: context_dir.to_path_buf(),
        commit: out.stdout.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use std::path::Path;

    fn git_ok(sha: &str) -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: format!("{sha}\n"),
            stderr: String::new(),
        }
    }

    #[tokio::test]
    async fn detects_a_containerfile_and_reads_the_commit() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Containerfile", "FROM alpine\n");
        let exec = FakeExecutor::new();
        exec.expect_call(
            "git",
            &["-C", "/repo", "rev-parse", "HEAD"],
            git_ok("abc123"),
        );

        let plan = detect(&exec, &fsys, Path::new("/repo"))
            .await
            .expect("detect");
        assert_eq!(plan.containerfile, Path::new("/repo/Containerfile"));
        assert_eq!(plan.context_dir, Path::new("/repo"));
        assert_eq!(plan.commit, "abc123");
    }

    #[tokio::test]
    async fn falls_back_to_a_dockerfile() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Dockerfile", "FROM alpine\n");
        let exec = FakeExecutor::new();
        exec.expect_call(
            "git",
            &["-C", "/repo", "rev-parse", "HEAD"],
            git_ok("def456"),
        );

        let plan = detect(&exec, &fsys, Path::new("/repo"))
            .await
            .expect("detect");
        assert_eq!(plan.containerfile, Path::new("/repo/Dockerfile"));
    }

    #[tokio::test]
    async fn rejects_a_repo_with_no_containerfile() {
        let fsys = FakeFileSystem::new();
        let exec = FakeExecutor::new();
        let err = detect(&exec, &fsys, Path::new("/repo")).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Containerfile"), "message was: {msg}");
        assert!(msg.contains("/repo"), "message was: {msg}");
    }

    #[tokio::test]
    async fn rejects_a_path_that_is_not_a_git_repo() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Containerfile", "FROM alpine\n");
        let exec = FakeExecutor::new();
        exec.expect_call(
            "git",
            &["-C", "/repo", "rev-parse", "HEAD"],
            CommandOutput {
                status: 128,
                stdout: String::new(),
                stderr: "fatal: not a git repository".into(),
            },
        );

        let err = detect(&exec, &fsys, Path::new("/repo")).await.unwrap_err();
        assert!(
            err.to_string().contains("git repository"),
            "message was: {err}"
        );
    }
}
