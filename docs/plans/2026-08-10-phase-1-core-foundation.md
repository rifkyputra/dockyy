# kuadrat Phase 1 — Core Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `kuadrat-core` crate to the point where a `WorkloadSpec` can be rendered to a Podman Quadlet unit, applied to a real host, and removed again — verified by a CLI binary.

**Architecture:** A transport-agnostic library crate (`kuadrat-core`) plus a thin CLI binary. All host interaction (`podman`, `systemctl`) goes through an `Executor` trait so the local implementation can be swapped for a fake in tests and, later, a remote one over SSH. Rendering is a pure function from spec to text, tested with golden files.

**Tech Stack:** Rust (edition 2021), tokio, anyhow, thiserror, serde/serde_json, async-trait, clap; dev-dependencies tempfile.

## Global Constraints

- **`core` never opens a socket and never knows about hosts.** If any `kuadrat-core` function grows a `host: &str` parameter, the design has failed. Network concerns belong to the daemon (phase 3).
- **Every host command goes through the `Executor` trait.** No direct `tokio::process::Command` calls outside `exec::local`.
- **The spec is the source of truth; unit files are derived artifacts** kuadrat owns and may overwrite.
- **Secret values never appear** in specs, logs, error messages, or committed files. Specs carry secret *names* only.
- **Paths are injectable.** No hardcoded `/etc/...` outside `Paths::default()`.
- `make check && make test` must pass with **zero warnings** before any task is considered done.
- Commit messages follow Conventional Commits and end with the trailer `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.
- Out of scope for phase 1 (do not build): SQLite persistence, deploy state machine, gateway/Caddy, secrets CRUD, logs, events, HTTP, MCP.

---

### Task 1: Repository and workspace scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/core/Cargo.toml`
- Create: `crates/core/src/lib.rs`
- Create: `crates/cli/Cargo.toml`
- Create: `crates/cli/src/main.rs`
- Create: `Makefile`
- Create: `.gitignore`
- Create: `README.md`

**Interfaces:**
- Consumes: nothing
- Produces: crate `kuadrat-core` (lib), crate `kuadrat` (bin), `make check`, `make test`

- [ ] **Step 1: Create the crate directories**

The repository already exists at `devbox/kuadrat` with its docs committed. From the repo root:

```bash
mkdir -p crates/core/src crates/cli/src
```

- [ ] **Step 2: Write the workspace root `Cargo.toml`**

```toml
[workspace]
members = ["crates/core", "crates/cli"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["fs", "process", "rt-multi-thread", "macros"] }
async-trait = "0.1"
clap = { version = "4", features = ["derive"] }
tempfile = "3"
```

- [ ] **Step 3: Write `crates/core/Cargo.toml`**

```toml
[package]
name = "kuadrat-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
async-trait.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 4: Write `crates/cli/Cargo.toml`**

```toml
[package]
name = "kuadrat"
version.workspace = true
edition.workspace = true
license.workspace = true

[[bin]]
name = "kuadrat"
path = "src/main.rs"

[dependencies]
kuadrat-core = { path = "../core" }
anyhow.workspace = true
tokio.workspace = true
clap.workspace = true
serde_json.workspace = true
```

- [ ] **Step 5: Write placeholder sources**

`crates/core/src/lib.rs`:

```rust
//! kuadrat-core — transport-agnostic engine for Podman Quadlet workloads.
//!
//! This crate never opens a socket and never takes a host parameter.
```

`crates/cli/src/main.rs`:

```rust
fn main() {
    println!("kuadrat");
}
```

- [ ] **Step 6: Write the `Makefile`**

```makefile
.PHONY: build check test fmt

build:
	cargo build

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

test:
	cargo test --all

fmt:
	cargo fmt
```

- [ ] **Step 7: Confirm `.gitignore` and `README.md` are already present**

Both were committed with the repo's docs. `.gitignore` should contain `/target`; add it if missing.

- [ ] **Step 8: Verify the workspace builds cleanly**

Run: `make build && make check`
Expected: builds with zero warnings, clippy clean.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "chore: scaffold cargo workspace with core and cli crates"
```

---

### Task 2: `spec` module — the workload description

**Files:**
- Create: `crates/core/src/spec.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct WorkloadSpec { name: String, image: String, command: Option<Vec<String>>, env: Vec<(String, String)>, ports: Vec<String>, volumes: Vec<String>, secrets: Vec<String>, memory_max: Option<String>, health_cmd: Option<String>, restart_policy: RestartPolicy }`
  - `pub enum RestartPolicy { Always, OnFailure, No }` with `fn as_systemd(&self) -> &'static str`
  - `pub fn slug(name: &str) -> String`
  - `impl WorkloadSpec { pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self }`

Note: a `route` field is deliberately absent — the gateway arrives in phase 2.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/spec.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_replaces_separators() {
        assert_eq!(slug("My App"), "my-app");
        assert_eq!(slug("pbrain_api"), "pbrain-api");
        assert_eq!(slug("Web--Server "), "web-server");
    }

    #[test]
    fn slug_strips_disallowed_characters() {
        assert_eq!(slug("app@v1.2"), "appv12");
    }

    #[test]
    fn restart_policy_maps_to_systemd_values() {
        assert_eq!(RestartPolicy::Always.as_systemd(), "always");
        assert_eq!(RestartPolicy::OnFailure.as_systemd(), "on-failure");
        assert_eq!(RestartPolicy::No.as_systemd(), "no");
    }

    #[test]
    fn new_spec_defaults_to_restart_always_and_empty_collections() {
        let spec = WorkloadSpec::new("pbrain", "docker.io/library/node:22-alpine");
        assert_eq!(spec.name, "pbrain");
        assert_eq!(spec.restart_policy, RestartPolicy::Always);
        assert!(spec.env.is_empty());
        assert!(spec.secrets.is_empty());
        assert!(spec.memory_max.is_none());
    }

    #[test]
    fn spec_round_trips_through_json() {
        let mut spec = WorkloadSpec::new("pbrain", "node:22-alpine");
        spec.ports.push("3000:3000".to_string());
        spec.secrets.push("db-password".to_string());
        let json = serde_json::to_string(&spec).expect("serialize");
        let back: WorkloadSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, back);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod spec;` to `crates/core/src/lib.rs`, then run:
`cargo test -p kuadrat-core spec`
Expected: FAIL — `cannot find function slug`, `cannot find type WorkloadSpec`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/spec.rs`:

```rust
use serde::{Deserialize, Serialize};

/// How systemd should restart the workload.
///
/// `#[default]` on the variant rather than a manual `impl Default` — clippy's
/// `derivable_impls` lint rejects the hand-written version under `-D warnings`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
    #[default]
    Always,
    OnFailure,
    No,
}

impl RestartPolicy {
    pub fn as_systemd(&self) -> &'static str {
        match self {
            RestartPolicy::Always => "always",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::No => "no",
        }
    }
}

/// Declarative description of one workload. The source of truth; unit files are derived.
///
/// `secrets` holds secret *names* only — values live in `podman secret` and never appear here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    pub name: String,
    pub image: String,
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub ports: Vec<String>,
    pub volumes: Vec<String>,
    pub secrets: Vec<String>,
    pub memory_max: Option<String>,
    pub health_cmd: Option<String>,
    pub restart_policy: RestartPolicy,
}

impl WorkloadSpec {
    pub fn new(name: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            image: image.into(),
            ..Default::default()
        }
    }

    /// Filesystem- and systemd-safe identifier derived from the name.
    pub fn slug(&self) -> String {
        slug(&self.name)
    }
}

/// Lowercase, collapse separators to `-`, drop anything else.
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if ch == ' ' || ch == '-' || ch == '_' {
            if !last_dash && !out.is_empty() {
                out.push('-');
                last_dash = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core spec`
Expected: 5 tests PASS.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/spec.rs crates/core/src/lib.rs
git commit -m "feat(core): add WorkloadSpec, RestartPolicy, and slug"
```

---

### Task 3: `exec` module — the executor seam

**Files:**
- Create: `crates/core/src/exec/mod.rs`
- Create: `crates/core/src/exec/local.rs`
- Create: `crates/core/src/exec/fake.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub struct CommandOutput { pub status: i32, pub stdout: String, pub stderr: String }` with `pub fn success(&self) -> bool`
  - `#[async_trait] pub trait Executor: Send + Sync { async fn run(&self, program: &str, args: &[String]) -> anyhow::Result<CommandOutput>; }`
  - `pub struct LocalExecutor;`
  - `pub struct FakeExecutor` with `pub fn new() -> Self`, `pub fn expect(&self, program: &str, output: CommandOutput)`, `pub fn calls(&self) -> Vec<(String, Vec<String>)>`

This is the most important abstraction in phase 1: it makes failure paths testable now and a remote executor possible later.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/exec/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::local::LocalExecutor;

    #[test]
    fn command_output_success_reflects_status() {
        let ok = CommandOutput { status: 0, stdout: String::new(), stderr: String::new() };
        let bad = CommandOutput { status: 1, stdout: String::new(), stderr: String::new() };
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
            CommandOutput { status: 0, stdout: "ok".into(), stderr: String::new() },
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
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod exec;` to `crates/core/src/lib.rs`, then run:
`cargo test -p kuadrat-core exec`
Expected: FAIL — modules `local` and `fake` do not exist.

- [ ] **Step 3: Write the trait and output type**

Prepend to `crates/core/src/exec/mod.rs`:

```rust
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
```

- [ ] **Step 4: Write `LocalExecutor`**

`crates/core/src/exec/local.rs`:

```rust
use anyhow::{Context, Result};
use async_trait::async_trait;
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
}
```

- [ ] **Step 5: Write `FakeExecutor`**

`crates/core/src/exec/fake.rs`:

```rust
use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use super::{CommandOutput, Executor};

/// Test double. Returns scripted output per program and records every call.
#[derive(Default)]
pub struct FakeExecutor {
    scripted: Mutex<HashMap<String, CommandOutput>>,
    calls: Mutex<Vec<(String, Vec<String>)>>,
}

impl FakeExecutor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the output returned for every invocation of `program`.
    pub fn expect(&self, program: &str, output: CommandOutput) {
        self.scripted
            .lock()
            .expect("scripted lock")
            .insert(program.to_string(), output);
    }

    /// Every (program, args) pair seen, in order.
    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait]
impl Executor for FakeExecutor {
    async fn run(&self, program: &str, args: &[String]) -> Result<CommandOutput> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((program.to_string(), args.to_vec()));

        self.scripted
            .lock()
            .expect("scripted lock")
            .get(program)
            .cloned()
            .ok_or_else(|| anyhow!("unexpected command: {program}"))
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core exec`
Expected: 4 tests PASS.

- [ ] **Step 7: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/exec crates/core/src/lib.rs
git commit -m "feat(core): add Executor trait with local and fake implementations"
```

---

### Task 4: `workloads::render` — spec to Quadlet unit

**Files:**
- Create: `crates/core/src/workloads/mod.rs`
- Create: `crates/core/src/workloads/render.rs`
- Create: `crates/core/tests/golden/minimal.container`
- Create: `crates/core/tests/golden/full.container`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `spec::{WorkloadSpec, RestartPolicy, slug}`
- Produces:
  - `pub const MANAGED_MARKER: &str = "# kuadrat-managed: true";`
  - `pub fn render(spec: &WorkloadSpec) -> String`
  - `pub fn container_name(spec: &WorkloadSpec) -> String` → `kuadrat-<slug>`

Rendering is a pure function — no I/O, no executor. Quadlet syntax errors are the likeliest bug class here, so every spec feature gets a golden case.

- [ ] **Step 1: Write the golden files**

`crates/core/tests/golden/minimal.container`:

```ini
# kuadrat-managed: true
[Unit]
Description=kuadrat workload pbrain

[Container]
Image=docker.io/library/node:22-alpine
ContainerName=kuadrat-pbrain

[Service]
Restart=always

[Install]
WantedBy=multi-user.target
```

`crates/core/tests/golden/full.container`:

```ini
# kuadrat-managed: true
[Unit]
Description=kuadrat workload pbrain api

[Container]
Image=docker.io/library/node:22-alpine
ContainerName=kuadrat-pbrain-api
PublishPort=3000:3000
PublishPort=9229:9229
Volume=/srv/pbrain:/app:Z
Environment=NODE_ENV=production
Environment=PORT=3000
Secret=db-password
HealthCmd=curl -fsS http://localhost:3000/health
Exec=node server.js --port 3000

[Service]
Restart=on-failure
MemoryMax=512M

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/workloads/render.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{RestartPolicy, WorkloadSpec};

    #[test]
    fn renders_minimal_spec() {
        let spec = WorkloadSpec::new("pbrain", "docker.io/library/node:22-alpine");
        let expected = include_str!("../../tests/golden/minimal.container");
        assert_eq!(render(&spec), expected);
    }

    #[test]
    fn renders_full_spec() {
        let mut spec = WorkloadSpec::new("pbrain api", "docker.io/library/node:22-alpine");
        spec.ports = vec!["3000:3000".into(), "9229:9229".into()];
        spec.volumes = vec!["/srv/pbrain:/app:Z".into()];
        spec.env = vec![
            ("NODE_ENV".into(), "production".into()),
            ("PORT".into(), "3000".into()),
        ];
        spec.secrets = vec!["db-password".into()];
        spec.health_cmd = Some("curl -fsS http://localhost:3000/health".into());
        spec.command = Some(vec!["node".into(), "server.js".into(), "--port".into(), "3000".into()]);
        spec.memory_max = Some("512M".into());
        spec.restart_policy = RestartPolicy::OnFailure;

        let expected = include_str!("../../tests/golden/full.container");
        assert_eq!(render(&spec), expected);
    }

    #[test]
    fn container_name_is_prefixed_slug() {
        let spec = WorkloadSpec::new("My App", "alpine");
        assert_eq!(container_name(&spec), "kuadrat-my-app");
    }

    #[test]
    fn rendered_unit_always_carries_the_managed_marker() {
        let spec = WorkloadSpec::new("x", "alpine");
        assert!(render(&spec).starts_with(MANAGED_MARKER));
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Create `crates/core/src/workloads/mod.rs` containing `pub mod render;`, add `pub mod workloads;` to `crates/core/src/lib.rs`, then run:
`cargo test -p kuadrat-core render`
Expected: FAIL — `cannot find function render`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/workloads/render.rs`:

```rust
use crate::spec::{slug, WorkloadSpec};

/// Marker identifying a unit file kuadrat generated and may overwrite.
pub const MANAGED_MARKER: &str = "# kuadrat-managed: true";

/// Container name kuadrat assigns to a workload.
pub fn container_name(spec: &WorkloadSpec) -> String {
    format!("kuadrat-{}", slug(&spec.name))
}

/// Render a spec to Quadlet `.container` unit text. Pure — no I/O.
pub fn render(spec: &WorkloadSpec) -> String {
    let mut out = String::new();

    out.push_str(MANAGED_MARKER);
    out.push('\n');

    out.push_str("[Unit]\n");
    out.push_str(&format!("Description=kuadrat workload {}\n\n", spec.name));

    out.push_str("[Container]\n");
    out.push_str(&format!("Image={}\n", spec.image));
    out.push_str(&format!("ContainerName={}\n", container_name(spec)));
    for port in &spec.ports {
        out.push_str(&format!("PublishPort={port}\n"));
    }
    for volume in &spec.volumes {
        out.push_str(&format!("Volume={volume}\n"));
    }
    for (key, value) in &spec.env {
        out.push_str(&format!("Environment={key}={value}\n"));
    }
    for secret in &spec.secrets {
        out.push_str(&format!("Secret={secret}\n"));
    }
    if let Some(health) = &spec.health_cmd {
        out.push_str(&format!("HealthCmd={health}\n"));
    }
    if let Some(command) = &spec.command {
        out.push_str(&format!("Exec={}\n", command.join(" ")));
    }
    out.push('\n');

    out.push_str("[Service]\n");
    out.push_str(&format!("Restart={}\n", spec.restart_policy.as_systemd()));
    if let Some(memory) = &spec.memory_max {
        out.push_str(&format!("MemoryMax={memory}\n"));
    }
    out.push('\n');

    out.push_str("[Install]\nWantedBy=multi-user.target\n");

    out
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core render`
Expected: 4 tests PASS. If the golden comparison fails, print both sides and reconcile whitespace exactly — trailing newlines matter.

- [ ] **Step 6: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/workloads crates/core/tests/golden crates/core/src/lib.rs
git commit -m "feat(core): render WorkloadSpec to Quadlet unit with golden tests"
```

---

### Task 5: `workloads::apply` — write, reload, start, remove

**Files:**
- Create: `crates/core/src/workloads/paths.rs`
- Create: `crates/core/src/workloads/apply.rs`
- Modify: `crates/core/src/workloads/mod.rs`

**Interfaces:**
- Consumes: `spec::WorkloadSpec`, `exec::{Executor, CommandOutput}`, `workloads::render::render`
- Produces:
  - `pub struct Paths { pub quadlet_dir: PathBuf }` with `Paths::default()` → `/etc/containers/systemd`, and `Paths::rooted(root: &Path)` for tests
  - `pub fn unit_path(paths: &Paths, spec_name: &str) -> PathBuf` → `<quadlet_dir>/<slug>.container`
  - `pub async fn apply(exec: &dyn Executor, paths: &Paths, spec: &WorkloadSpec) -> anyhow::Result<()>`
  - `pub async fn remove(exec: &dyn Executor, paths: &Paths, name: &str) -> anyhow::Result<()>`

`apply` writes the unit, runs `systemctl daemon-reload`, then `systemctl start <slug>`. `remove` stops the unit, deletes the file, and reloads.

- [ ] **Step 1: Write `paths.rs`**

```rust
use std::path::{Path, PathBuf};

use crate::spec::slug;

/// Filesystem locations kuadrat writes to. Injectable so tests never touch `/etc`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub quadlet_dir: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            quadlet_dir: PathBuf::from("/etc/containers/systemd"),
        }
    }
}

impl Paths {
    /// All paths relative to `root` — for tests and dry runs.
    pub fn rooted(root: &Path) -> Self {
        Self {
            quadlet_dir: root.join("containers/systemd"),
        }
    }
}

/// Path of the generated unit file for a workload name.
pub fn unit_path(paths: &Paths, spec_name: &str) -> PathBuf {
    paths.quadlet_dir.join(format!("{}.container", slug(spec_name)))
}
```

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/workloads/apply.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::spec::WorkloadSpec;
    use tempfile::tempdir;

    fn ok() -> CommandOutput {
        CommandOutput { status: 0, stdout: String::new(), stderr: String::new() }
    }

    #[tokio::test]
    async fn apply_writes_unit_reloads_and_starts() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply succeeds");

        let written = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("unit written");
        assert!(written.contains("Image=alpine"));

        let calls = fake.calls();
        assert_eq!(calls[0].1, vec!["daemon-reload".to_string()]);
        assert_eq!(calls[1].1, vec!["start".to_string(), "pbrain".to_string()]);
    }

    #[tokio::test]
    async fn apply_is_idempotent_for_the_same_spec() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("first apply");
        let first = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");
        apply(&fake, &paths, &spec).await.expect("second apply");
        let second = std::fs::read_to_string(unit_path(&paths, "pbrain")).expect("read");

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn apply_fails_when_daemon_reload_fails() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect(
            "systemctl",
            CommandOutput { status: 1, stdout: String::new(), stderr: "bad unit".into() },
        );

        let spec = WorkloadSpec::new("pbrain", "alpine");
        let err = apply(&fake, &paths, &spec).await.unwrap_err();
        assert!(err.to_string().contains("daemon-reload"));
    }

    #[tokio::test]
    async fn remove_stops_unit_and_deletes_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply");
        remove(&fake, &paths, "pbrain").await.expect("remove");

        assert!(!unit_path(&paths, "pbrain").exists());
    }

    #[tokio::test]
    async fn remove_is_ok_when_unit_absent() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        remove(&fake, &paths, "never-existed").await.expect("no error");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Add to `crates/core/src/workloads/mod.rs`:

```rust
pub mod apply;
pub mod paths;
pub mod render;
```

Run: `cargo test -p kuadrat-core apply`
Expected: FAIL — `cannot find function apply`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/workloads/apply.rs`:

```rust
use anyhow::{bail, Context, Result};
use tokio::fs;

use crate::exec::Executor;
use crate::spec::{slug, WorkloadSpec};
use crate::workloads::render::render;

pub use crate::workloads::paths::{unit_path, Paths};

/// Write the unit, reload systemd, and start the workload.
///
/// Idempotent: the same spec produces byte-identical output.
pub async fn apply(exec: &dyn Executor, paths: &Paths, spec: &WorkloadSpec) -> Result<()> {
    fs::create_dir_all(&paths.quadlet_dir)
        .await
        .with_context(|| format!("creating {}", paths.quadlet_dir.display()))?;

    let path = unit_path(paths, &spec.name);
    fs::write(&path, render(spec))
        .await
        .with_context(|| format!("writing {}", path.display()))?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;
    systemctl(exec, &["start".to_string(), slug(&spec.name)]).await?;

    Ok(())
}

/// Stop the workload, delete its unit, and reload systemd. Safe if absent.
pub async fn remove(exec: &dyn Executor, paths: &Paths, name: &str) -> Result<()> {
    let path = unit_path(paths, name);

    if path.exists() {
        systemctl(exec, &["stop".to_string(), slug(name)]).await?;
        fs::remove_file(&path)
            .await
            .with_context(|| format!("removing {}", path.display()))?;
        systemctl(exec, &["daemon-reload".to_string()]).await?;
    }

    Ok(())
}

async fn systemctl(exec: &dyn Executor, args: &[String]) -> Result<()> {
    let out = exec.run("systemctl", args).await?;
    if !out.success() {
        bail!("systemctl {} failed: {}", args.join(" "), out.stderr.trim());
    }
    Ok(())
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core apply`
Expected: 5 tests PASS.

- [ ] **Step 6: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/workloads
git commit -m "feat(core): apply and remove workloads through the executor"
```

---

### Task 6: `workloads::query` — status of a workload

**Files:**
- Create: `crates/core/src/workloads/query.rs`
- Modify: `crates/core/src/workloads/mod.rs`

**Interfaces:**
- Consumes: `exec::Executor`, `workloads::paths::{Paths, unit_path}`, `spec::slug`
- Produces:
  - `pub enum WorkloadState { Running, Stopped, Failed, NotInstalled, Unknown }` with `pub fn label(&self) -> &'static str`
  - `pub async fn status(exec: &dyn Executor, paths: &Paths, name: &str) -> anyhow::Result<WorkloadState>`
  - `pub async fn list(paths: &Paths) -> anyhow::Result<Vec<String>>` — names of kuadrat-managed units

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/workloads/query.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::workloads::apply::{apply, Paths};
    use crate::spec::WorkloadSpec;
    use tempfile::tempdir;

    fn out(stdout: &str) -> CommandOutput {
        CommandOutput { status: 0, stdout: stdout.into(), stderr: String::new() }
    }

    #[tokio::test]
    async fn status_is_not_installed_without_a_unit_file() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();

        let state = status(&fake, &paths, "absent").await.expect("status");
        assert_eq!(state, WorkloadState::NotInstalled);
    }

    #[tokio::test]
    async fn status_maps_systemctl_output() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());

        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));
        let spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &paths, &spec).await.expect("apply");

        assert_eq!(
            status(&fake, &paths, "pbrain").await.expect("status"),
            WorkloadState::Running
        );

        let fake2 = FakeExecutor::new();
        fake2.expect("systemctl", out("failed\n"));
        assert_eq!(
            status(&fake2, &paths, "pbrain").await.expect("status"),
            WorkloadState::Failed
        );

        let fake3 = FakeExecutor::new();
        fake3.expect("systemctl", out("inactive\n"));
        assert_eq!(
            status(&fake3, &paths, "pbrain").await.expect("status"),
            WorkloadState::Stopped
        );
    }

    #[tokio::test]
    async fn list_returns_only_kuadrat_managed_units() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fake = FakeExecutor::new();
        fake.expect("systemctl", out("active\n"));

        apply(&fake, &paths, &WorkloadSpec::new("alpha", "alpine"))
            .await
            .expect("apply alpha");
        apply(&fake, &paths, &WorkloadSpec::new("beta", "alpine"))
            .await
            .expect("apply beta");

        std::fs::write(paths.quadlet_dir.join("foreign.container"), "[Container]\n")
            .expect("write foreign unit");

        let mut names = list(&paths).await.expect("list");
        names.sort();
        assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod query;` to `crates/core/src/workloads/mod.rs`, then run:
`cargo test -p kuadrat-core query`
Expected: FAIL — `cannot find function status`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/workloads/query.rs`:

```rust
use anyhow::{Context, Result};
use tokio::fs;

use crate::exec::Executor;
use crate::spec::slug;
use crate::workloads::paths::{unit_path, Paths};
use crate::workloads::render::MANAGED_MARKER;

/// Runtime state of a workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadState {
    Running,
    Stopped,
    Failed,
    NotInstalled,
    Unknown,
}

impl WorkloadState {
    pub fn label(&self) -> &'static str {
        match self {
            WorkloadState::Running => "Running",
            WorkloadState::Stopped => "Stopped",
            WorkloadState::Failed => "Failed",
            WorkloadState::NotInstalled => "Not installed",
            WorkloadState::Unknown => "Unknown",
        }
    }
}

/// Current state of a workload. `NotInstalled` when no unit file exists.
pub async fn status(exec: &dyn Executor, paths: &Paths, name: &str) -> Result<WorkloadState> {
    if !unit_path(paths, name).exists() {
        return Ok(WorkloadState::NotInstalled);
    }

    let out = exec
        .run("systemctl", &["is-active".to_string(), slug(name)])
        .await?;

    Ok(match out.stdout.trim() {
        "active" => WorkloadState::Running,
        "inactive" => WorkloadState::Stopped,
        "failed" => WorkloadState::Failed,
        _ => WorkloadState::Unknown,
    })
}

/// Names of every kuadrat-managed workload found in the quadlet directory.
pub async fn list(paths: &Paths) -> Result<Vec<String>> {
    let mut names = Vec::new();

    let mut entries = match fs::read_dir(&paths.quadlet_dir).await {
        Ok(entries) => entries,
        Err(_) => return Ok(names),
    };

    while let Some(entry) = entries
        .next_entry()
        .await
        .with_context(|| format!("reading {}", paths.quadlet_dir.display()))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("container") {
            continue;
        }
        let content = match fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(_) => continue,
        };
        if !content.starts_with(MANAGED_MARKER) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            names.push(stem.to_string());
        }
    }

    Ok(names)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core query`
Expected: 3 tests PASS.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/workloads
git commit -m "feat(core): query workload status and list managed units"
```

---

### Task 7: `kuadrat` CLI — the phase-1 acceptance

**Files:**
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: everything from `kuadrat-core`
- Produces: `kuadrat apply <file.json>`, `kuadrat remove <name>`, `kuadrat status <name>`, `kuadrat list`

This binary is how phase 1 is verified: a spec applied to a real host and removed again.

- [ ] **Step 1: Write the CLI**

`crates/cli/src/main.rs`:

```rust
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use kuadrat_core::exec::local::LocalExecutor;
use kuadrat_core::spec::WorkloadSpec;
use kuadrat_core::workloads::apply::{apply, remove, Paths};
use kuadrat_core::workloads::query::{list, status};

#[derive(Parser)]
#[command(name = "kuadrat", about = "Podman Quadlet deployment for a single host")]
struct Cli {
    /// Treat all paths as relative to this root (for testing without touching /etc)
    #[arg(long)]
    root: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Apply a workload spec from a JSON file
    Apply { file: std::path::PathBuf },
    /// Remove a workload by name
    Remove { name: String },
    /// Show a workload's state
    Status { name: String },
    /// List kuadrat-managed workloads
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = match &cli.root {
        Some(root) => Paths::rooted(root),
        None => Paths::default(),
    };
    let exec = LocalExecutor;

    match cli.command {
        Command::Apply { file } => {
            let text = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let spec: WorkloadSpec =
                serde_json::from_str(&text).context("parsing spec JSON")?;
            apply(&exec, &paths, &spec).await?;
            println!("applied {}", spec.name);
        }
        Command::Remove { name } => {
            remove(&exec, &paths, &name).await?;
            println!("removed {name}");
        }
        Command::Status { name } => {
            let state = status(&exec, &paths, &name).await?;
            println!("{}", state.label());
        }
        Command::List => {
            for name in list(&paths).await? {
                println!("{name}");
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 2: Build and verify the CLI compiles**

Run: `make build && make check`
Expected: builds clean, zero warnings.

- [ ] **Step 3: Verify against a temp root (no root privileges needed)**

```bash
mkdir -p /tmp/kuadrat-test
cat > /tmp/kuadrat-test/pbrain.json <<'EOF'
{
  "name": "pbrain",
  "image": "docker.io/library/alpine:3",
  "command": ["sleep", "3600"],
  "env": [],
  "ports": [],
  "volumes": [],
  "secrets": [],
  "memory_max": "128M",
  "health_cmd": null,
  "restart_policy": "Always"
}
EOF
cargo run -p kuadrat -- --root /tmp/kuadrat-test apply /tmp/kuadrat-test/pbrain.json
cat /tmp/kuadrat-test/containers/systemd/pbrain.container
```

Expected: the rendered unit prints with `Image=docker.io/library/alpine:3` and `MemoryMax=128M`. `systemctl` will fail here unless run as root — that is expected at this step; the file write is what is being verified.

- [ ] **Step 4: Verify on a real host**

On a machine with podman and systemd, as root:

```bash
sudo cargo run -p kuadrat -- apply /tmp/kuadrat-test/pbrain.json
sudo systemctl status pbrain
sudo podman ps --filter name=kuadrat-pbrain
cargo run -p kuadrat -- list
cargo run -p kuadrat -- status pbrain
sudo cargo run -p kuadrat -- remove pbrain
```

Expected: `systemctl status pbrain` shows active, `podman ps` shows `kuadrat-pbrain`, `list` prints `pbrain`, `status` prints `Running`, and after `remove` the unit file is gone and the container stopped.

**This is the phase-1 done criterion.** If `systemctl status` reports a unit-file parse error, the golden files in Task 4 encode invalid Quadlet syntax — fix the renderer and the goldens together.

- [ ] **Step 5: Run the full test suite**

Run: `make check && make test`
Expected: all tests pass, zero warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/cli/src/main.rs
git commit -m "feat(cli): add apply, remove, status, and list commands"
```

---

## Phase 1 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] A spec applied on a real host produces a running container under systemd
- [ ] `remove` leaves no unit file and no running container
- [ ] No `kuadrat-core` function takes a host parameter
- [ ] No direct `Command::new` outside `exec::local`
- [ ] Repo added as a submodule at the devbox root with a `docs/INDEX.md` row and capability card

## What phase 2 adds

The deploy state machine (`Detect → Build → Secrets → Apply → Route → Healthcheck`), SQLite persistence making the spec the durable source of truth, the Caddy gateway, `podman secret` management, per-stage compensation, the concurrency lock, and crash reconciliation. `WorkloadSpec` gains a `route` field there.
