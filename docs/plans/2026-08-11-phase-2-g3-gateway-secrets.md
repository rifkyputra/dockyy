# kuadrat Phase 2 · G3 — Gateway + Secrets Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the two side-effect subsystems the deploy loop needs — a Caddy fragment per app (route → TLS) and `podman secret` management whose values travel by stdin, never argv — plus the `Executor::run_with_stdin` method that makes the secret path safe.

**Architecture:** `run_with_stdin` extends the existing `Executor` seam so a secret value is piped to `podman` rather than placed in a world-readable command line. The `FakeExecutor` records the call but never the stdin. The Caddy gateway reuses phase-1's ownership-guard pattern (refuse to touch a file kuadrat did not write), extracted into a shared `managed` module so it is not duplicated. Both reach the host only through the two seams.

**Tech Stack:** Rust (edition 2021), anyhow, tokio, async-trait, existing `exec`/`fs` seams, clap. Runtime: `podman` (≥ 4.7 for `secret create --replace`; this host is 4.9.3), `systemctl`, Caddy.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.**
- **Every host command goes through the `Executor` trait; every file access through `FileSystem`.** No `tokio::process::Command` outside `exec::local`; no `std::fs`/`tokio::fs`/`Path::exists()` outside `fs::local` (the store carve-out excepted). `podman` and `systemctl` are invoked via `exec.run(...)`/`exec.run_with_stdin(...)`.
- **Secret VALUES never appear in argv, in a spec, in a log, in an error message, or in `FakeExecutor::calls()`.** They travel only through stdin. Error messages name the secret and echo podman's stderr, never the value.
- Paths are injectable: no hardcoded `/etc/...` or `/var/...` outside `Paths::default()`.
- **Do not build, in G3** (later groups): the state-machine driver, compensation, restart-on-change, reconciliation, or any wiring of detect/build/store/gateway/secrets together — that is G4. G3 adds the subsystems in isolation.

---

### Task 1: `Executor::run_with_stdin`

**Files:**
- Modify: `crates/core/src/exec/mod.rs`
- Modify: `crates/core/src/exec/local.rs`
- Modify: `crates/core/src/exec/fake.rs`

**Interfaces:**
- Consumes: the existing `Executor` trait, `CommandOutput`
- Produces:
  - trait method `async fn run_with_stdin(&self, program: &str, args: &[String], stdin: &str) -> Result<CommandOutput>` with a default impl that bails
  - `LocalExecutor` override that pipes `stdin` to the child
  - `FakeExecutor` override that records `(program, args)` in `calls()` (NOT the stdin) and the stdin in a separate `stdins()` accessor
  - `FakeExecutor::stdins(&self) -> Vec<String>`

The default-bail means a future executor (e.g. SSH) compiles without this method until it opts in.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/exec/mod.rs`:

```rust
    fn ok_out(stdout: &str) -> CommandOutput {
        CommandOutput { status: 0, stdout: stdout.into(), stderr: String::new() }
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
        assert!(!flat.contains("the-secret-value"), "call log leaked the secret: {flat}");

        // The dedicated accessor exposes it for the one test that needs it.
        assert_eq!(fake.stdins(), vec!["the-secret-value".to_string()]);
    }

    #[tokio::test]
    async fn default_run_with_stdin_bails() {
        struct Bare;
        #[async_trait::async_trait]
        impl Executor for Bare {
            async fn run(&self, _program: &str, _args: &[String]) -> anyhow::Result<CommandOutput> {
                Ok(CommandOutput { status: 0, stdout: String::new(), stderr: String::new() })
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
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core exec 2>&1 | grep -E 'error|no method|no function'
```
Expected: FAIL — `no method named run_with_stdin` / `no method named stdins`.

- [ ] **Step 3: Add the trait method with a default**

In `crates/core/src/exec/mod.rs`, inside the `#[async_trait]` trait, after `run`:

```rust
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
```

- [ ] **Step 4: Implement it on `LocalExecutor`**

In `crates/core/src/exec/local.rs`, add to the `impl Executor for LocalExecutor` block. Add `use tokio::io::AsyncWriteExt;` at the top of the file:

```rust
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
```

- [ ] **Step 5: Implement it on `FakeExecutor`, and refactor `run` to share the lookup**

In `crates/core/src/exec/fake.rs`, add a `stdins` field to the struct:

```rust
    stdins: Mutex<Vec<String>>,
```

Add two private helpers and refactor both trait methods to use them. Replace the existing `run` impl and add `run_with_stdin`:

```rust
impl FakeExecutor {
    // ... existing new/expect/expect_call/calls ...

    fn record_call(&self, program: &str, args: &[String]) {
        self.calls
            .lock()
            .expect("calls lock")
            .push((program.to_string(), args.to_vec()));
    }

    fn scripted(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        let key = (program.to_string(), args.to_vec());
        if let Some(output) = self.by_call.lock().expect("by_call lock").get(&key).cloned() {
            return Ok(output);
        }
        self.by_program
            .lock()
            .expect("by_program lock")
            .get(program)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected command: {program} {}", args.join(" ")))
    }

    /// Stdin values received, in order. Deliberately separate from `calls()` so
    /// a secret value is never in the general call log.
    pub fn stdins(&self) -> Vec<String> {
        self.stdins.lock().expect("stdins lock").clone()
    }
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        self.record_call(program, args);
        self.scripted(program, args)
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[String],
        stdin: &str,
    ) -> Result<CommandOutput> {
        self.record_call(program, args);
        self.stdins.lock().expect("stdins lock").push(stdin.to_string());
        self.scripted(program, args)
    }
}
```

Note: `FakeExecutor` derives `Default`, so the new `stdins` field needs no manual init — confirm the struct still `#[derive(Default)]`s (it does) and that `Mutex<Vec<String>>: Default` (it is).

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core exec
```
Expected: all exec tests PASS (existing + 3 new).

- [ ] **Step 7: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/exec/
git commit -m "feat(core): add Executor::run_with_stdin for secret values"
```

---

### Task 2: `secrets` module

**Files:**
- Create: `crates/core/src/secrets/mod.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `exec::Executor` (incl. `run_with_stdin`)
- Produces:
  - `secrets::set(exec: &dyn Executor, name: &str, value: &str) -> Result<()>` — `podman secret create --replace <name> -`, value via stdin (upsert)
  - `secrets::list(exec: &dyn Executor) -> Result<Vec<String>>` — `podman secret ls --format {{.Name}}`
  - `secrets::remove(exec: &dyn Executor, name: &str) -> Result<()>` — `podman secret rm <name>`
  - `secrets::ensure_all(exec: &dyn Executor, names: &[String]) -> Result<()>` — bail naming any missing secret

`--replace` requires podman ≥ 4.7; this host is 4.9.3. `ensure_all` is the Secrets stage's pre-flight check (G4 runs it before Apply, so a missing credential fails while the old version still serves).

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/secrets/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput { status: 0, stdout: stdout.into(), stderr: String::new() }
    }

    #[tokio::test]
    async fn set_passes_the_value_by_stdin_never_argv() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["secret", "create", "--replace", "db-pw", "-"], ok(""));

        set(&exec, "db-pw", "supersecret").await.expect("set");

        // Value went through stdin...
        assert_eq!(exec.stdins(), vec!["supersecret".to_string()]);
        // ...and never into the argv log.
        let flat = format!("{:?}", exec.calls());
        assert!(!flat.contains("supersecret"), "argv leaked the secret: {flat}");
    }

    #[tokio::test]
    async fn set_fails_without_echoing_the_value() {
        let exec = FakeExecutor::new();
        exec.expect_call(
            "podman",
            &["secret", "create", "--replace", "db-pw", "-"],
            CommandOutput { status: 125, stdout: String::new(), stderr: "already in use".into() },
        );
        let err = set(&exec, "db-pw", "supersecret").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("db-pw"), "message was: {msg}");
        assert!(!msg.contains("supersecret"), "error leaked the value: {msg}");
    }

    #[tokio::test]
    async fn list_parses_names() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["secret", "ls", "--format", "{{.Name}}"], ok("alpha\nbeta\n"));
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

        ensure_all(&exec, &["alpha".to_string()]).await.expect("present");

        let err = ensure_all(&exec, &["alpha".to_string(), "gamma".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("gamma"), "message was: {err}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod secrets;` to `crates/core/src/lib.rs` (alphabetical: after `managed` if present, else between `fs` and `spec` — put it before `spec`). Then:
```bash
cargo test -p kuadrat-core secrets 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function set`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/secrets/mod.rs`:

```rust
//! `podman secret` management. Specs carry secret NAMES; values travel only
//! through stdin (via `Executor::run_with_stdin`), never argv, never a log.

use std::collections::HashSet;

use anyhow::{bail, Result};

use crate::exec::Executor;

/// Create or replace a secret. The value is piped to podman; it never appears
/// in argv. Errors name the secret and echo podman's stderr, never the value.
pub async fn set(exec: &dyn Executor, name: &str, value: &str) -> Result<()> {
    let out = exec
        .run_with_stdin(
            "podman",
            &[
                "secret".to_string(),
                "create".to_string(),
                "--replace".to_string(),
                name.to_string(),
                "-".to_string(),
            ],
            value,
        )
        .await?;
    if !out.success() {
        bail!("podman secret create failed for {name}: {}", out.stderr.trim());
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
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core secrets
```
Expected: 5 tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/secrets/ crates/core/src/lib.rs
git commit -m "feat(core): podman secret management, values via stdin only"
```

---

### Task 3: extract the ownership guard into a `managed` module

**Files:**
- Create: `crates/core/src/managed.rs`
- Modify: `crates/core/src/workloads/apply.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `fs::FileSystem`
- Produces: `pub(crate) async fn managed::ensure_owned(fsys: &dyn FileSystem, path: &Path, marker: &str, action: &str) -> Result<bool>` — `Ok(true)` if the file exists and starts with `marker`, `Ok(false)` if absent, error if a file is there without the marker

This is a pure refactor: `ensure_owned` currently lives privately in `apply.rs` and hardcodes `MANAGED_MARKER`. The gateway (Task 4) needs the same guard, so it moves to a shared module and takes the marker as a parameter. Behavior for `apply`/`remove` is unchanged — their existing tests must stay green.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/managed.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::fake::FakeFileSystem;
    use std::path::Path;

    const MARKER: &str = "# kuadrat-managed: true";

    #[tokio::test]
    async fn absent_file_is_not_owned() {
        let fsys = FakeFileSystem::new();
        assert!(!ensure_owned(&fsys, Path::new("/x/a"), MARKER, "overwrite").await.unwrap());
    }

    #[tokio::test]
    async fn a_file_with_the_marker_is_owned() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/x/a", "# kuadrat-managed: true\nrest\n");
        assert!(ensure_owned(&fsys, Path::new("/x/a"), MARKER, "overwrite").await.unwrap());
    }

    #[tokio::test]
    async fn a_foreign_file_is_refused() {
        let fsys = FakeFileSystem::new();
        fsys.insert("/x/a", "hand written\n");
        let err = ensure_owned(&fsys, Path::new("/x/a"), MARKER, "remove").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("remove"), "message was: {msg}");
        assert!(msg.contains("/x/a"), "message was: {msg}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod managed;` to `crates/core/src/lib.rs` (alphabetical: between `fs` and `secrets`). Then:
```bash
cargo test -p kuadrat-core managed 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function ensure_owned`.

- [ ] **Step 3: Write the shared function**

Prepend to `crates/core/src/managed.rs`:

```rust
//! The ownership guard shared by everything that writes marker-tagged files
//! (unit files, Caddy fragments): refuse to overwrite or delete a file kuadrat
//! did not write, so a hand-authored config is never silently clobbered.

use std::path::Path;

use anyhow::{bail, Result};

use crate::fs::FileSystem;

/// `Ok(true)` when the file exists and starts with `marker`, `Ok(false)` when
/// it is absent, and an error when a file is present that does not carry the
/// marker (so kuadrat did not write it).
pub(crate) async fn ensure_owned(
    fsys: &dyn FileSystem,
    path: &Path,
    marker: &str,
    action: &str,
) -> Result<bool> {
    if !fsys.exists(path).await? {
        return Ok(false);
    }

    let existing = fsys.read_to_string(path).await?;
    if !existing.starts_with(marker) {
        bail!(
            "refusing to {action} {}: the file exists but does not start with `{marker}`, \
             so kuadrat did not write it; resolve the drift by hand",
            path.display()
        );
    }

    Ok(true)
}
```

- [ ] **Step 4: Point `apply.rs` at the shared function**

In `crates/core/src/workloads/apply.rs`:
1. Delete the private `async fn ensure_owned(...)` function entirely.
2. Add `use crate::managed::ensure_owned;` to the imports.
3. Update the two call sites to pass the marker. They currently read
   `ensure_owned(fsys, &path, "overwrite")` and `ensure_owned(fsys, &path, "remove")`;
   change them to `ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite")` and
   `ensure_owned(fsys, &path, MANAGED_MARKER, "remove")`. `MANAGED_MARKER` is already
   imported in `apply.rs` (from `crate::workloads::render`).

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core managed && cargo test -p kuadrat-core apply
```
Expected: 3 new `managed` tests PASS, and all existing `apply` tests still PASS (behavior unchanged).

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean. (No `dead_code` on `ensure_owned` — `apply` uses it now, `gateway` will too.)

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/managed.rs crates/core/src/workloads/apply.rs crates/core/src/lib.rs
git commit -m "refactor(core): extract the ownership guard into a shared managed module"
```

---

### Task 4: `gateway` module — a Caddy fragment per app

**Files:**
- Create: `crates/core/src/gateway/mod.rs`
- Create: `crates/core/tests/golden/route.caddy`
- Modify: `crates/core/src/workloads/paths.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `exec::Executor`, `fs::FileSystem`, `managed::ensure_owned`, `workloads::render::MANAGED_MARKER`, `workloads::paths::Paths`
- Produces:
  - `gateway::Route { pub domain: String, pub port: u16 }`
  - `gateway::render_fragment(route: &Route) -> String` — the Caddy fragment (pure)
  - `gateway::fragment_path(paths: &Paths, slug: &str) -> PathBuf` → `<caddy_dir>/<slug>.caddy`
  - `gateway::apply_route(exec, fsys, paths, slug, route) -> Result<()>` — write the fragment (guarded), reload Caddy
  - `gateway::remove_route(exec, fsys, paths, slug) -> Result<()>` — delete the fragment (guarded), reload Caddy
  - new `Paths` field `pub caddy_dir: PathBuf` (default `/etc/caddy/kuadrat.d`, rooted `<root>/caddy/kuadrat.d`)

The operator's Caddyfile carries one line, `import kuadrat.d/*.caddy`; kuadrat owns each `<slug>.caddy`. Caddy auto-provisions TLS for a public domain.

- [ ] **Step 1: Add the golden fragment and the `caddy_dir` path**

Create `crates/core/tests/golden/route.caddy` — note the single TAB before `reverse_proxy`, and exactly one trailing newline after `}`:

```
# kuadrat-managed: true
example.com {
	reverse_proxy localhost:3000
}
```

In `crates/core/src/workloads/paths.rs`, add the field to the struct and both constructors:

```rust
pub struct Paths {
    pub quadlet_dir: PathBuf,
    pub db_path: PathBuf,
    pub caddy_dir: PathBuf,
}
```
`Default`: `caddy_dir: PathBuf::from("/etc/caddy/kuadrat.d")`.
`rooted`: `caddy_dir: root.join("caddy/kuadrat.d")`.

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/gateway/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use crate::workloads::paths::Paths;
    use std::path::Path;

    fn route() -> Route {
        Route { domain: "example.com".to_string(), port: 3000 }
    }
    fn ok() -> CommandOutput {
        CommandOutput { status: 0, stdout: String::new(), stderr: String::new() }
    }

    #[test]
    fn renders_the_golden_fragment() {
        let expected = include_str!("../../tests/golden/route.caddy");
        assert_eq!(render_fragment(&route()), expected);
    }

    #[test]
    fn fragment_path_is_slug_dot_caddy_under_caddy_dir() {
        let paths = Paths::rooted(Path::new("/root"));
        assert_eq!(
            fragment_path(&paths, "web"),
            Path::new("/root/caddy/kuadrat.d/web.caddy")
        );
    }

    #[tokio::test]
    async fn apply_route_writes_the_fragment_and_reloads_caddy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["reload", "caddy"], ok());

        apply_route(&exec, &fsys, &paths, "web", &route()).await.expect("apply");

        let written = std::fs::read_to_string(fragment_path(&paths, "web")).expect("fragment");
        assert!(written.contains("reverse_proxy localhost:3000"));
        assert!(written.starts_with(crate::workloads::render::MANAGED_MARKER));
        assert_eq!(exec.calls()[0].1, vec!["reload".to_string(), "caddy".to_string()]);
    }

    #[tokio::test]
    async fn apply_route_refuses_a_foreign_fragment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();

        std::fs::create_dir_all(&paths.caddy_dir).expect("mkdir");
        std::fs::write(fragment_path(&paths, "web"), "hand written\n").expect("foreign");

        let err = apply_route(&exec, &fsys, &paths, "web", &route()).await.unwrap_err();
        assert!(err.to_string().contains("did not write it"), "message was: {err}");
    }

    #[tokio::test]
    async fn remove_route_deletes_and_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["reload", "caddy"], ok());

        apply_route(&exec, &fsys, &paths, "web", &route()).await.expect("apply");
        remove_route(&exec, &fsys, &paths, "web").await.expect("remove");
        assert!(!fragment_path(&paths, "web").exists());
    }

    #[tokio::test]
    async fn remove_route_is_ok_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fsys = crate::fs::local::LocalFileSystem;
        let exec = FakeExecutor::new();
        remove_route(&exec, &fsys, &paths, "never").await.expect("no error");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Add `pub mod gateway;` to `crates/core/src/lib.rs` (alphabetical: between `fs` and `managed`). Then:
```bash
cargo test -p kuadrat-core gateway 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function render_fragment`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/gateway/mod.rs`:

```rust
//! One Caddy fragment per app. kuadrat writes `<caddy_dir>/<slug>.caddy`; the
//! operator's Caddyfile imports them with `import kuadrat.d/*.caddy`. Each
//! fragment is marker-guarded so kuadrat never clobbers a hand-written file.

use std::path::PathBuf;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::managed::ensure_owned;
use crate::workloads::paths::Paths;
use crate::workloads::render::MANAGED_MARKER;

/// A public route: a domain reverse-proxied to a local port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub domain: String,
    pub port: u16,
}

/// Render the Caddy fragment for a route. Pure — no I/O. Caddy auto-provisions
/// TLS for a public domain.
pub fn render_fragment(route: &Route) -> String {
    format!(
        "{MANAGED_MARKER}\n{} {{\n\treverse_proxy localhost:{}\n}}\n",
        route.domain, route.port
    )
}

/// Path of the fragment kuadrat writes for an app.
pub fn fragment_path(paths: &Paths, slug: &str) -> PathBuf {
    paths.caddy_dir.join(format!("{slug}.caddy"))
}

/// Write the fragment (refusing to clobber a foreign file) and reload Caddy.
pub async fn apply_route(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    slug: &str,
    route: &Route,
) -> Result<()> {
    let path = fragment_path(paths, slug);
    ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite").await?;

    fsys.create_dir_all(&paths.caddy_dir).await?;
    fsys.write(&path, &render_fragment(route)).await?;

    reload_caddy(exec).await
}

/// Delete the fragment (if kuadrat owns it) and reload Caddy. Safe if absent.
pub async fn remove_route(
    exec: &dyn Executor,
    fsys: &dyn FileSystem,
    paths: &Paths,
    slug: &str,
) -> Result<()> {
    let path = fragment_path(paths, slug);
    if !ensure_owned(fsys, &path, MANAGED_MARKER, "remove").await? {
        return Ok(());
    }
    fsys.remove_file(&path).await?;
    reload_caddy(exec).await
}

async fn reload_caddy(exec: &dyn Executor) -> Result<()> {
    let out = exec
        .run("systemctl", &["reload".to_string(), "caddy".to_string()])
        .await?;
    if !out.success() {
        bail!("systemctl reload caddy failed: {}", out.stderr.trim());
    }
    Ok(())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core gateway
```
Expected: 6 tests PASS. If the golden comparison fails, print both sides and reconcile whitespace exactly — the tab before `reverse_proxy` and the single trailing newline both matter.

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/gateway/ crates/core/tests/golden/route.caddy crates/core/src/workloads/paths.rs crates/core/src/lib.rs
git commit -m "feat(core): Caddy fragment per app with the ownership guard"
```

---

### Task 5: `kuadrat secret` CLI + secrets acceptance

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `scripts/secrets-acceptance.sh`

**Interfaces:**
- Consumes: `secrets::{set, list, remove}`, `exec::local::LocalExecutor`
- Produces: `kuadrat secret set <name>` (value from stdin), `kuadrat secret ls`, `kuadrat secret rm <name>`

Proves the secret round-trips through real podman with the value never on the command line. The gateway's real-host proof (a live route over TLS) waits for G5's full deploy acceptance — it needs Caddy plus a public domain.

- [ ] **Step 1: Add the `secret` subcommand**

In `crates/cli/src/main.rs`, add a variant to the `Command` enum and a sub-enum:

```rust
    /// Manage podman secrets (values read from stdin, never argv)
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
```

```rust
#[derive(Subcommand)]
enum SecretAction {
    /// Create or replace a secret; the value is read verbatim from stdin
    Set { name: String },
    /// List secret names
    Ls,
    /// Remove a secret
    Rm { name: String },
}
```

And the match arm in `main` (reuse the existing `exec`):

```rust
        Command::Secret { action } => {
            use kuadrat_core::secrets;
            match action {
                SecretAction::Set { name } => {
                    use std::io::Read;
                    let mut value = String::new();
                    std::io::stdin()
                        .read_to_string(&mut value)
                        .context("reading the secret value from stdin")?;
                    secrets::set(&exec, &name, &value).await?;
                    println!("set secret {name}");
                }
                SecretAction::Ls => {
                    for n in secrets::list(&exec).await? {
                        println!("{n}");
                    }
                }
                SecretAction::Rm { name } => {
                    secrets::remove(&exec, &name).await?;
                    println!("removed secret {name}");
                }
            }
        }
```

The value is never printed and never placed in argv — only the name is echoed.

- [ ] **Step 2: Build and verify the CLI compiles**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && make check
```
Expected: builds clean, zero warnings.

- [ ] **Step 3: Write the acceptance script**

Create `scripts/secrets-acceptance.sh`:

```bash
#!/usr/bin/env bash
# kuadrat G3 secrets acceptance. Run as your normal user (rootless podman):
#   bash scripts/secrets-acceptance.sh
# Expects the release binary:  cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
NAME=kuadrat-g3-test
VALUE="top-secret-value-$$"

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }
cleanup() { podman secret rm "$NAME" >/dev/null 2>&1; }
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found. Build it: PATH=\$HOME/.cargo/bin:\$PATH cargo build --release"; exit 1; }

echo "kuadrat G3 secrets acceptance"
echo "binary : $BIN"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"

# Start clean.
podman secret rm "$NAME" >/dev/null 2>&1

echo "== set (value via stdin)"
printf %s "$VALUE" | "$BIN" secret set "$NAME"
rc=$?
[ $rc -eq 0 ] && ok "secret set exited 0" || bad "secret set exited $rc"

echo "== kuadrat and podman both see it"
"$BIN" secret ls | grep -qx "$NAME" && ok "kuadrat secret ls shows $NAME" || bad "kuadrat ls missing $NAME"
podman secret ls --format '{{.Name}}' | grep -qx "$NAME" && ok "podman secret ls shows $NAME" || bad "podman ls missing $NAME"

echo "== set --replace is idempotent"
printf %s "$VALUE-v2" | "$BIN" secret set "$NAME" && ok "re-set (replace) exited 0" || bad "re-set failed"

echo "== rm"
"$BIN" secret rm "$NAME" >/dev/null && ok "secret rm exited 0" || bad "secret rm failed"
podman secret ls --format '{{.Name}}' | grep -qx "$NAME" && bad "secret still present after rm" || ok "secret gone after rm"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  G3 SECRETS ACCEPTANCE: PASS" || echo "  G3 SECRETS ACCEPTANCE: FAIL"
exit $fail
```

Make it executable: `chmod +x scripts/secrets-acceptance.sh`.

- [ ] **Step 4: Run the acceptance on this host**

podman rootless secrets need no sudo. Run:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
bash scripts/secrets-acceptance.sh
```

Expected: `G3 SECRETS ACCEPTANCE: PASS`, all checks green. Paste the full output into your report.

- [ ] **Step 5: Run the whole suite and the gate**

```bash
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: `make check` clean; every test-result line shows `0 failed`.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs scripts/secrets-acceptance.sh
git commit -m "feat(cli): add kuadrat secret; G3 secrets acceptance passes on a real host"
```

---

## G3 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] Secret values reach podman only via stdin — never in argv, a spec, a log, an error, or `FakeExecutor::calls()`
- [ ] `gateway` and `secrets` reach the host only through `Executor`/`FileSystem` — no direct `Command`, `std::fs`, or `tokio::fs`
- [ ] The Caddy fragment is marker-guarded (refuses to clobber a hand-written file), reusing the shared `managed::ensure_owned` — not a second copy
- [ ] A secret round-trips through real podman, proven by `scripts/secrets-acceptance.sh`
- [ ] `apply`/`remove` still pass after the `ensure_owned` extraction (behavior unchanged)

## Not in G3 (later groups)

The state-machine driver that calls detect → build → secrets → apply → route → healthcheck, with per-stage compensation, restart-on-change, and the concurrency lock (G4); reconciliation and the full deploy acceptance including a live route over TLS (G5). A `route` field on `WorkloadSpec` (referencing `gateway::Route`) is added in G4 when the machine reads it from a spec — G3 defines `Route` where it is used and does not touch the spec.
