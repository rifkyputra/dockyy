# kuadrat Phase 2 · G2 — Detect + Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn a local repo path into a built, tagged container image — `detect` finds the Containerfile and the git commit, `build` runs `podman build` — verified on a real host by a `kuadrat build` command.

**Architecture:** Two pure-ish functions in the existing `deploy` module, both driven through the phase-1 `Executor` seam (`git`, `podman`) and `FileSystem` seam (file probes). No new seams, no working-directory parameter — `git -C <dir>` and `podman build <context>` take their directory as an argument. A thin CLI subcommand wires them to the local implementations for acceptance.

**Tech Stack:** Rust (edition 2021), anyhow, existing `exec`/`fs` seams, clap. Runtime: `git`, `podman`.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.**
- **Every host command goes through the `Executor` trait; every file probe through `FileSystem`.** No `tokio::process::Command` outside `exec::local`; no `std::fs`/`tokio::fs`/`Path::exists()` in `deploy` code. `git` and `podman` are invoked via `exec.run(...)`.
- **Do not build, in G2** (later groups): the gateway, secrets, `run_with_stdin`, the state-machine driver, compensation, reconciliation, or the SQLite deploy flow. G2 is detect + build + a build CLI only. Do not wire detect/build into the store — that is G4.
- The `deploy` module already holds `Stage` and `DeployStatus` (from G1). Add `detect` and `build` as submodules; leave the enums untouched.

---

### Task 1: `detect` — repo path to BuildPlan

**Files:**
- Create: `crates/core/src/deploy/detect.rs`
- Modify: `crates/core/src/deploy/mod.rs`

**Interfaces:**
- Consumes: `exec::Executor`, `fs::FileSystem`
- Produces:
  - `deploy::detect::BuildPlan { pub containerfile: PathBuf, pub context_dir: PathBuf, pub commit: String }`
  - `deploy::detect::detect(exec: &dyn Executor, fsys: &dyn FileSystem, context_dir: &Path) -> Result<BuildPlan>`

`detect` looks for `Containerfile` then `Dockerfile` in `context_dir`, then reads `git -C <dir> rev-parse HEAD` for the tag. It fails if neither file is present, or if the path is not a git repo.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/deploy/detect.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use std::path::Path;

    fn git_ok(sha: &str) -> CommandOutput {
        CommandOutput { status: 0, stdout: format!("{sha}\n"), stderr: String::new() }
    }

    #[tokio::test]
    async fn detects_a_containerfile_and_reads_the_commit() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Containerfile", "FROM alpine\n");
        let exec = FakeExecutor::new();
        exec.expect_call("git", &["-C", "/repo", "rev-parse", "HEAD"], git_ok("abc123"));

        let plan = detect(&exec, &fsys, Path::new("/repo")).await.expect("detect");
        assert_eq!(plan.containerfile, Path::new("/repo/Containerfile"));
        assert_eq!(plan.context_dir, Path::new("/repo"));
        assert_eq!(plan.commit, "abc123");
    }

    #[tokio::test]
    async fn falls_back_to_a_dockerfile() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Dockerfile", "FROM alpine\n");
        let exec = FakeExecutor::new();
        exec.expect_call("git", &["-C", "/repo", "rev-parse", "HEAD"], git_ok("def456"));

        let plan = detect(&exec, &fsys, Path::new("/repo")).await.expect("detect");
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
            CommandOutput { status: 128, stdout: String::new(), stderr: "fatal: not a git repository".into() },
        );

        let err = detect(&exec, &fsys, Path::new("/repo")).await.unwrap_err();
        assert!(err.to_string().contains("git repository"), "message was: {err}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add to `crates/core/src/deploy/mod.rs` (below the existing enums): `pub mod detect;`. Then:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core detect 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function detect`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/deploy/detect.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core detect
```
Expected: 4 tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/detect.rs crates/core/src/deploy/mod.rs
git commit -m "feat(core): detect a repo's Containerfile and HEAD commit"
```

---

### Task 2: `build` — BuildPlan to a tagged image

**Files:**
- Create: `crates/core/src/deploy/build.rs`
- Modify: `crates/core/src/deploy/mod.rs`

**Interfaces:**
- Consumes: `exec::Executor`, `deploy::detect::BuildPlan`
- Produces:
  - `deploy::build::image_reference(slug: &str, commit: &str) -> String` → `localhost/kuadrat-<slug>:<commit>`
  - `deploy::build::build(exec: &dyn Executor, plan: &BuildPlan, slug: &str) -> Result<String>` — runs `podman build`, returns the image reference

The `localhost/` prefix marks the image as local-only, so Quadlet's `Image=` never tries to pull it. `build` computes the reference from `slug` + the plan's commit, so there is one place that knows the naming convention.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/deploy/build.rs` with only the tests:

```rust
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
        CommandOutput { status: 0, stdout: String::new(), stderr: String::new() }
    }

    #[test]
    fn image_reference_is_namespaced_and_local() {
        assert_eq!(image_reference("web", "abc123"), "localhost/kuadrat-web:abc123");
    }

    #[tokio::test]
    async fn build_invokes_podman_and_returns_the_reference() {
        let exec = FakeExecutor::new();
        exec.expect_call(
            "podman",
            &["build", "-t", "localhost/kuadrat-web:abc123", "-f", "/repo/Containerfile", "/repo"],
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
            &["build", "-t", "localhost/kuadrat-web:abc123", "-f", "/repo/Containerfile", "/repo"],
            CommandOutput { status: 1, stdout: String::new(), stderr: "build step failed".into() },
        );

        let err = build(&exec, &plan(), "web").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("build"), "message was: {msg}");
        assert!(msg.contains("build step failed"), "message was: {msg}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod build;` to `crates/core/src/deploy/mod.rs`. Then:
```bash
cargo test -p kuadrat-core 'deploy::build' 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function build`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/deploy/build.rs`:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core 'deploy::build'
```
Expected: 3 tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/build.rs crates/core/src/deploy/mod.rs
git commit -m "feat(core): build a tagged image from a BuildPlan"
```

---

### Task 3: `kuadrat build` CLI + real-host acceptance

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `scripts/build-acceptance.sh`

**Interfaces:**
- Consumes: `deploy::detect::detect`, `deploy::build::build`, `spec::slug`, `exec::local::LocalExecutor`, `fs::local::LocalFileSystem`
- Produces: `kuadrat build <path>` — detects, builds, and prints the image reference

The app name is the directory's basename, slugified. This is a diagnostic ("build the image without deploying"); the full deploy loop arrives in G4.

- [ ] **Step 1: Add the `Build` subcommand**

In `crates/cli/src/main.rs`, add a variant to the `Command` enum:

```rust
    /// Build a repo's image without deploying it
    Build { path: std::path::PathBuf },
```

And a match arm in `main` (alongside the existing `apply`/`remove`/`status`/`list` arms). Note the CLI already constructs a `LocalExecutor`; add a `LocalFileSystem` for detect:

```rust
        Command::Build { path } => {
            use kuadrat_core::deploy::{build::build, detect::detect};
            use kuadrat_core::fs::local::LocalFileSystem;
            use kuadrat_core::spec::slug;

            let fsys = LocalFileSystem;
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .context("path has no final component to name the app after")?;
            let plan = detect(&exec, &fsys, &path).await?;
            let image = build(&exec, &plan, &slug(name)).await?;
            println!("{image}");
        }
```

If `exec` is not already in scope in the match (it is — the other arms use it), reuse it. Ensure `use anyhow::Context;` is present at the top of the file (the existing `apply` arm already uses `.context(...)`, so it should be).

- [ ] **Step 2: Build and verify the CLI compiles**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && make check
```
Expected: builds clean, zero warnings.

- [ ] **Step 3: Write the acceptance script**

Create `scripts/build-acceptance.sh` (mirrors `scripts/acceptance.sh`'s style — pass/fail counters, self-cleaning):

```bash
#!/usr/bin/env bash
# kuadrat G2 build acceptance. Run as your normal user (podman rootless is fine):
#   bash scripts/build-acceptance.sh
# Expects the release binary built:  cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
WORK=$(mktemp -d)
APP=g2demo
SLUG=g2demo
IMAGE="localhost/kuadrat-${SLUG}"

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }

[ -x "$BIN" ] || { echo "FATAL: $BIN not found. Build it: PATH=\$HOME/.cargo/bin:\$PATH cargo build --release"; exit 1; }

echo "kuadrat G2 build acceptance"
echo "binary : $BIN"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"
echo "workdir: $WORK/${APP}"

# A tiny git repo with a Containerfile.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
RUN echo "kuadrat g2 build" > /built.txt
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm "init"
SHA=$(git -C "$WORK/$APP" rev-parse HEAD)

echo "== build"
OUT=$("$BIN" build "$WORK/$APP" 2>&1); rc=$?
echo "$OUT"
[ $rc -eq 0 ] && ok "build exited 0" || bad "build exited $rc"
[ "$OUT" = "${IMAGE}:${SHA}" ] && ok "printed reference matches localhost/kuadrat-<slug>:<sha>" || bad "reference was '$OUT', expected '${IMAGE}:${SHA}'"

echo "== podman sees the image"
podman image exists "${IMAGE}:${SHA}" && ok "image ${IMAGE}:${SHA} exists" || bad "image not found"

echo "== detect rejects a non-repo"
mkdir -p "$WORK/norepo"
cp "$WORK/$APP/Containerfile" "$WORK/norepo/"
"$BIN" build "$WORK/norepo" >/dev/null 2>&1 && bad "build of a non-repo should fail" || ok "build of a non-repo fails"

echo "== detect rejects a repo with no Containerfile"
mkdir -p "$WORK/nocf"; git -C "$WORK/nocf" init -q
"$BIN" build "$WORK/nocf" >/dev/null 2>&1 && bad "build with no Containerfile should fail" || ok "build with no Containerfile fails"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"

# Clean up the built image and the workdir.
podman rmi -f "${IMAGE}:${SHA}" >/dev/null 2>&1
rm -rf "$WORK"
[ $fail -eq 0 ] && echo "  G2 BUILD ACCEPTANCE: PASS" || echo "  G2 BUILD ACCEPTANCE: FAIL"
exit $fail
```

Make it executable: `chmod +x scripts/build-acceptance.sh`.

- [ ] **Step 4: Run the acceptance on this host**

podman IS installed here (4.9.3), and rootless build works without sudo. Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
bash scripts/build-acceptance.sh
```

Expected: `G2 BUILD ACCEPTANCE: PASS`, all checks green. Paste the full output into your report. If `podman build` fails with a pull error on `alpine:3`, run `podman pull docker.io/library/alpine:3` once and re-run — the image may not be in the rootless store yet.

- [ ] **Step 5: Run the whole suite and the gate**

```bash
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: `make check` clean; every test-result line shows `0 failed`.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs scripts/build-acceptance.sh
git commit -m "feat(cli): add kuadrat build; G2 build acceptance passes on a real host"
```

---

## G2 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] `detect` and `build` reach the host only through `Executor`/`FileSystem` — no direct `Command`, `std::fs`, or `tokio::fs` in `deploy`
- [ ] A repo path yields a tagged image, proven by `scripts/build-acceptance.sh` on a real host
- [ ] The `localhost/` prefix is on every image reference (so Quadlet never tries to pull a local build)
- [ ] No store, gateway, secrets, or state-machine code was added

## Not in G2 (later groups)

Gateway + secrets + `run_with_stdin` (G3); the state-machine driver that calls `detect`/`build`/store together, compensation, restart-on-change (G4); reconciliation + the full deploy acceptance (G5). The `kuadrat build` command is a diagnostic; `kuadrat deploy` (which runs detect and build as stages and records them in the store) is G4.
