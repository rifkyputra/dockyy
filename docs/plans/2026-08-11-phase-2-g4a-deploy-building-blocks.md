# kuadrat Phase 2 · G4a — Deploy Building Blocks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pieces the deploy state machine will compose — the `route` field and its validation, `%`-escaping in rendered values, the deploy value types (`DeployError`/`DeployOutcome`/`Ctx`), restart-on-change in apply, and the healthcheck stage — without yet wiring them into a driver.

**Architecture:** G4 (the state machine) is split into G4a (these building blocks) and G4b (the driver + compensation + `kuadrat deploy` + acceptance). G4a adds no new orchestration; every piece is independently testable against the existing seams. The machine in G4b calls them in order.

**Tech Stack:** Rust (edition 2021), anyhow, thiserror, tokio (adds the `time` feature), serde, existing `exec`/`fs`/`store` seams.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- Commit messages follow Conventional Commits and end with the trailer `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>` — exactly "Claude Opus 5".
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.**
- **Every host command goes through the `Executor` trait; every file access through `FileSystem`.** No `tokio::process::Command` outside `exec::local`; no `std::fs`/`tokio::fs`/`Path::exists()` in non-test code outside `fs::local` (store carve-out excepted).
- **Do not build, in G4a** (that is G4b): the `deploy::run` driver, the compensation matrix, the per-app-lock lifecycle wiring, `kuadrat deploy`, or `resolve_spec`. G4a defines the value types and stage helpers; it does not sequence them.

---

### Task 1: `route` on the spec + its validation

**Files:**
- Modify: `crates/core/src/spec.rs`
- Modify: `crates/core/src/gateway/mod.rs`

**Interfaces:**
- Consumes: nothing new
- Produces:
  - `spec::Route { pub domain: String, pub port: u16 }` (moved from `gateway`, now serde-serializable)
  - `WorkloadSpec` gains `pub route: Option<Route>`
  - `validate()` rejects a spec with `route.is_some()` and `health_cmd.is_none()`
  - `gateway` now imports `spec::Route` (its own `Route` def removed)

`Route` belongs on the spec (it's a field of the workload), so it moves out of `gateway`; `gateway` operates on `spec::Route`. A routed workload must declare a health check — public traffic must not reach a service with no readiness signal.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/spec.rs`:

```rust
    #[test]
    fn validate_rejects_a_route_without_a_health_cmd() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route { domain: "example.com".into(), port: 3000 });
        let err = spec.validate().unwrap_err();
        assert!(err.to_string().contains("health_cmd"), "message was: {err}");
    }

    #[test]
    fn validate_accepts_a_route_with_a_health_cmd() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route { domain: "example.com".into(), port: 3000 });
        spec.health_cmd = Some("curl -fsS http://localhost:3000/health".into());
        spec.validate().expect("valid");
    }

    #[test]
    fn a_spec_with_a_route_round_trips_through_json() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.route = Some(Route { domain: "example.com".into(), port: 3000 });
        let json = serde_json::to_string(&spec).expect("serialize");
        let back: WorkloadSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, back);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core spec 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find type Route` / no field `route`.

- [ ] **Step 3: Move `Route` into `spec.rs` and add the field**

In `crates/core/src/spec.rs`, add the `Route` type (near the top, after the imports):

```rust
/// A public route: a domain reverse-proxied to a local port. Rendered into a
/// Caddy fragment by the `gateway` module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub domain: String,
    pub port: u16,
}
```

Add the field to `WorkloadSpec` (after `restart_policy`, or anywhere in the struct — but keep it last for a clean diff):

```rust
    pub route: Option<Route>,
```

`WorkloadSpec` derives `Default`, so `route` defaults to `None` with no manual change.

In `validate()`, before the final `Ok(())`, add:

```rust
        if self.route.is_some() && self.health_cmd.is_none() {
            bail!(
                "workload {:?} declares a route but no health_cmd: public traffic \
                 must not reach a service with no readiness check",
                self.name
            );
        }
        if let Some(route) = &self.route {
            single_line("route domain", &route.domain)?;
        }
```

- [ ] **Step 4: Point `gateway` at `spec::Route`**

In `crates/core/src/gateway/mod.rs`:
1. Delete the local `pub struct Route { ... }` definition.
2. Add `use crate::spec::Route;` to the imports.
3. The test module constructs `Route { ... }` — it resolves via the module's `use crate::spec::Route;` (the tests use `super::*`, which now re-exports the imported `Route`). If the gateway test module has its own `Route` reference that no longer resolves, add `use crate::spec::Route;` inside the test module. Confirm `cargo test -p kuadrat-core gateway` still passes.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core spec && cargo test -p kuadrat-core gateway
```
Expected: new spec tests PASS, all gateway tests still PASS.

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/spec.rs crates/core/src/gateway/mod.rs
git commit -m "feat(core): move Route onto WorkloadSpec; a route requires a health_cmd"
```

---

### Task 2: escape `%` in rendered values

**Files:**
- Modify: `crates/core/src/workloads/render.rs`

**Interfaces:**
- Consumes: nothing new
- Produces: `render` escapes a literal `%` to `%%` in every `Environment=` key/value and every `Exec=` argument

systemd/Quadlet expand `%` specifiers (`%H`, `%i`, …) in directive values, so a literal `%` in an env value (a password, a URL-encoded string) would be silently mangled. Escaping `%` → `%%` makes it literal. `HealthCmd=` is deliberately left unescaped — a health command may use `%` intentionally.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/workloads/render.rs`:

```rust
    #[test]
    fn a_percent_in_an_env_value_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.env = vec![("PW".into(), "a%b".into())];
        let unit = render(&spec).expect("render");
        assert!(unit.contains("Environment=PW=a%%b"), "unit was:\n{unit}");
    }

    #[test]
    fn a_percent_in_an_exec_arg_is_escaped() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.command = Some(vec!["printf".into(), "100%".into()]);
        let unit = render(&spec).expect("render");
        assert!(unit.contains("100%%"), "unit was:\n{unit}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core render 2>&1 | grep -E 'FAILED|panicked|assert'
```
Expected: FAIL — the values render as `a%b` / `100%`, not `a%%b` / `100%%`.

- [ ] **Step 3: Add the escape and apply it**

In `crates/core/src/workloads/render.rs`, add a helper near `quote_exec_arg`:

```rust
/// Escape a literal `%` so systemd/Quadlet does not treat it as a specifier.
fn escape_percent(s: &str) -> String {
    s.replace('%', "%%")
}
```

In the env loop, escape key and value:

```rust
    for (key, value) in &spec.env {
        out.push_str(&format!(
            "Environment={}={}\n",
            escape_percent(key),
            escape_percent(value)
        ));
    }
```

In the `Exec=` construction, escape each argument before quoting:

```rust
    if let Some(command) = &spec.command {
        let argv: Vec<String> = command
            .iter()
            .map(|a| quote_exec_arg(&escape_percent(a)))
            .collect();
        out.push_str(&format!("Exec={}\n", argv.join(" ")));
    }
```

The existing golden files contain no `%`, so `escape_percent` is the identity on them and the golden tests still pass.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core render
```
Expected: all render tests PASS (new ones plus the unchanged goldens).

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/workloads/render.rs
git commit -m "fix(core): escape % in rendered Environment and Exec values"
```

---

### Task 3: deploy value types — `DeployError`, `DeployOutcome`, `Ctx`

**Files:**
- Modify: `crates/core/src/deploy/mod.rs`

**Interfaces:**
- Consumes: `Stage` (existing), `exec::Executor`, `fs::FileSystem`, `store::Store`, `workloads::paths::Paths`
- Produces:
  - `deploy::DeployError { pub stage: Stage, pub message: String }` — `thiserror`-derived, `Display` is `deploy failed at {stage:?}: {message}`
  - `deploy::DeployOutcome` — `Done { image: String } | RolledBack { failed_at: Stage, cause: String } | Failed { failed_at: Stage, cause: String }`
  - `deploy::Ctx<'a> { pub exec: &'a dyn Executor, pub fsys: &'a dyn FileSystem, pub store: &'a Store, pub paths: &'a Paths }`

`Ctx` bundles the four things every stage needs so stage functions do not take four arguments each. `DeployError` carries the stage so G4b's driver — and phase-4's agent — can report *where* a deploy failed, not just a log dump. Using `thiserror` here also resolves the "declared but unused" dependency noted in `known-gaps.md`.

- [ ] **Step 1: Write the failing test**

Add to a `#[cfg(test)] mod tests` in `crates/core/src/deploy/mod.rs` (the module already has one for the enums — add these tests to it):

```rust
    #[test]
    fn deploy_error_names_its_stage() {
        let err = DeployError { stage: Stage::Healthcheck, message: "timed out".into() };
        let shown = err.to_string();
        assert!(shown.contains("Healthcheck"), "shown: {shown}");
        assert!(shown.contains("timed out"), "shown: {shown}");
    }

    #[test]
    fn deploy_outcome_variants_carry_their_data() {
        let done = DeployOutcome::Done { image: "localhost/kuadrat-web:abc".into() };
        let rolled = DeployOutcome::RolledBack { failed_at: Stage::Route, cause: "caddy".into() };
        assert_ne!(done, rolled);
        if let DeployOutcome::RolledBack { failed_at, .. } = rolled {
            assert_eq!(failed_at, Stage::Route);
        } else {
            panic!("wrong variant");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core deploy:: 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find type DeployError`.

- [ ] **Step 3: Write the types**

Add to `crates/core/src/deploy/mod.rs` (after the existing enums, before the tests). Add the imports at the top of the file:

```rust
use crate::exec::Executor;
use crate::fs::FileSystem;
use crate::store::Store;
use crate::workloads::paths::Paths;

/// A deploy failure, tagged with the stage it happened in.
#[derive(Debug, thiserror::Error)]
#[error("deploy failed at {stage:?}: {message}")]
pub struct DeployError {
    pub stage: Stage,
    pub message: String,
}

/// The terminal state of a deploy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployOutcome {
    /// Healthcheck passed. The only success.
    Done { image: String },
    /// A stage failed and compensation restored the previous version.
    RolledBack { failed_at: Stage, cause: String },
    /// A stage failed and compensation ALSO failed — host state is unknown.
    Failed { failed_at: Stage, cause: String },
}

/// Everything a deploy stage needs, bundled so stages take one argument.
pub struct Ctx<'a> {
    pub exec: &'a dyn Executor,
    pub fsys: &'a dyn FileSystem,
    pub store: &'a Store,
    pub paths: &'a Paths,
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core deploy::
```
Expected: PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean. `Ctx`/`DeployOutcome`/`DeployError` are `pub` API surface, so they are not flagged dead even though G4a does not call them yet.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/mod.rs
git commit -m "feat(core): add DeployError, DeployOutcome, and the Ctx bundle"
```

---

### Task 4: apply restarts on change, starts otherwise

**Files:**
- Modify: `crates/core/src/workloads/apply.rs`

**Interfaces:**
- Consumes: existing `apply`
- Produces: `apply` issues `systemctl restart` when it overwrote an existing unit with *different* content; `systemctl start` when the unit is new or unchanged

Phase-1 `apply` always ran `systemctl start`, which is a no-op on an already-running unit — so a redeploy with a new image wrote a new unit file while the old container kept running. Now: new unit or unchanged → `start` (idempotent); changed → `daemon-reload` then `restart`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/workloads/apply.rs`:

```rust
    #[tokio::test]
    async fn redeploying_a_changed_spec_restarts_not_starts() {
        let dir = tempdir().expect("tempdir");
        let paths = Paths::rooted(dir.path());
        let fs = LocalFileSystem;
        let fake = FakeExecutor::new();
        fake.expect("systemctl", ok());

        // First apply: new unit → start.
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        apply(&fake, &fs, &paths, &spec).await.expect("first apply");

        // Second apply: different image → the unit content changes → restart.
        spec.image = "alpine:3.20".to_string();
        apply(&fake, &fs, &paths, &spec).await.expect("second apply");

        let calls = fake.calls();
        // The final systemctl action must be a restart of the changed unit.
        let restarted = calls
            .iter()
            .any(|(_, a)| a == &vec!["restart".to_string(), "kuadrat-pbrain".to_string()]);
        assert!(restarted, "expected a restart of the changed unit; calls: {calls:?}");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test -p kuadrat-core redeploying_a_changed_spec 2>&1 | grep -E 'FAILED|panicked|assert'
```
Expected: FAIL — apply always issues `start`, never `restart`.

- [ ] **Step 3: Change `apply`'s start logic**

In `crates/core/src/workloads/apply.rs`, in `apply`, replace the block that writes the unit and starts it. The current code is:

```rust
    ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite").await?;

    fsys.create_dir_all(&paths.quadlet_dir).await?;
    fsys.write(&path, &unit).await?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;
    systemctl(exec, &["start".to_string(), unit_name(&spec.name)]).await?;
```

Change it to read the previous content first, then choose start vs restart:

```rust
    ensure_owned(fsys, &path, MANAGED_MARKER, "overwrite").await?;

    let previous = if fsys.exists(&path).await? {
        Some(fsys.read_to_string(&path).await?)
    } else {
        None
    };

    fsys.create_dir_all(&paths.quadlet_dir).await?;
    fsys.write(&path, &unit).await?;

    systemctl(exec, &["daemon-reload".to_string()]).await?;

    // A new or byte-identical unit only needs `start` (a no-op if already
    // running). A changed unit needs `restart`, or the old container keeps
    // running behind the new unit file.
    let changed = matches!(&previous, Some(p) if p != &unit);
    let action = if changed { "restart" } else { "start" };
    systemctl(exec, &[action.to_string(), unit_name(&spec.name)]).await?;
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core apply
```
Expected: the new test PASSES, and all existing apply tests still PASS (a first apply and an unchanged re-apply both still issue `start`, so `apply_writes_unit_reloads_and_starts` and `apply_is_idempotent_for_the_same_spec` are unaffected).

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/workloads/apply.rs
git commit -m "feat(core): apply restarts a changed unit instead of a no-op start"
```

---

### Task 5: the healthcheck stage

**Files:**
- Create: `crates/core/src/deploy/health.rs`
- Modify: `crates/core/src/deploy/mod.rs`
- Modify: `Cargo.toml` (workspace `tokio` features)

**Interfaces:**
- Consumes: `exec::Executor`, `spec::WorkloadSpec`
- Produces: `deploy::health::healthcheck(exec: &dyn Executor, spec: &WorkloadSpec, slug: &str) -> Result<()>`

A workload with a `health_cmd` is polled via `podman healthcheck run kuadrat-<slug>` until it reports healthy or a 60s budget elapses. A workload without one falls back to `systemctl is-active`. This is what makes "a started unit is not a successful deploy" real.

- [ ] **Step 1: Add the `time` feature to tokio**

In the workspace root `Cargo.toml`, add `"time"` to the `tokio` feature list (it currently has `fs, process, rt-multi-thread, macros, io-util`):

```toml
tokio = { version = "1", features = ["fs", "process", "rt-multi-thread", "macros", "io-util", "time"] }
```

`tokio::time::sleep` (the poll interval) needs it.

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/deploy/health.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::spec::WorkloadSpec;
    use std::time::Duration;

    fn out(status: i32, stdout: &str) -> CommandOutput {
        CommandOutput { status, stdout: stdout.into(), stderr: String::new() }
    }

    #[tokio::test]
    async fn healthcheck_polls_podman_when_a_health_cmd_is_set() {
        let mut spec = WorkloadSpec::new("web", "alpine");
        spec.health_cmd = Some("curl -fsS localhost/health".into());
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(0, ""));

        healthcheck(&exec, &spec, "web").await.expect("healthy");
    }

    #[tokio::test]
    async fn poll_health_bails_after_the_attempt_budget() {
        let exec = FakeExecutor::new();
        exec.expect_call("podman", &["healthcheck", "run", "kuadrat-web"], out(1, ""));
        let err = poll_health(&exec, "kuadrat-web", 2, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("healthy"), "message was: {err}");
    }

    #[tokio::test]
    async fn healthcheck_without_a_health_cmd_uses_is_active() {
        let spec = WorkloadSpec::new("worker", "alpine"); // no health_cmd
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["is-active", "kuadrat-worker"], out(0, "active\n"));

        healthcheck(&exec, &spec, "worker").await.expect("active");
    }

    #[tokio::test]
    async fn healthcheck_without_a_health_cmd_bails_when_inactive() {
        let spec = WorkloadSpec::new("worker", "alpine");
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["is-active", "kuadrat-worker"], out(3, "failed\n"));

        let err = healthcheck(&exec, &spec, "worker").await.unwrap_err();
        assert!(err.to_string().contains("active"), "message was: {err}");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Add `pub mod health;` to `crates/core/src/deploy/mod.rs`. Then:
```bash
cargo test -p kuadrat-core health 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function healthcheck`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/deploy/health.rs`:

```rust
//! The healthcheck stage. A workload with a `health_cmd` is polled via
//! `podman healthcheck run` until healthy or the budget elapses; one without
//! falls back to `systemctl is-active`.

use std::time::Duration;

use anyhow::{bail, Result};

use crate::exec::Executor;
use crate::spec::WorkloadSpec;

const HEALTH_ATTEMPTS: u32 = 30;
const HEALTH_INTERVAL: Duration = Duration::from_secs(2);

/// Wait for a freshly-applied workload to be healthy. Uses the container's
/// podman healthcheck when the spec defines one, else `systemctl is-active`.
pub async fn healthcheck(exec: &dyn Executor, spec: &WorkloadSpec, slug: &str) -> Result<()> {
    let container = format!("kuadrat-{slug}");
    if spec.health_cmd.is_some() {
        poll_health(exec, &container, HEALTH_ATTEMPTS, HEALTH_INTERVAL).await
    } else {
        let out = exec
            .run("systemctl", &["is-active".to_string(), container.clone()])
            .await?;
        if out.stdout.trim() == "active" {
            Ok(())
        } else {
            bail!("{container} is not active after start (is-active: {})", out.stdout.trim());
        }
    }
}

/// Poll `podman healthcheck run <container>` until it succeeds or `attempts`
/// checks have failed, sleeping `interval` between checks.
async fn poll_health(
    exec: &dyn Executor,
    container: &str,
    attempts: u32,
    interval: Duration,
) -> Result<()> {
    for attempt in 0..attempts {
        let out = exec
            .run(
                "podman",
                &["healthcheck".to_string(), "run".to_string(), container.to_string()],
            )
            .await?;
        if out.success() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(interval).await;
        }
    }
    bail!("{container} did not become healthy after {attempts} checks");
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core health
```
Expected: 4 tests PASS quickly (the budget test uses a 1ms interval and 2 attempts).

- [ ] **Step 6: Run the whole suite and the gate**

```bash
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: `make check` clean; every test-result line shows `0 failed`.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/deploy/health.rs crates/core/src/deploy/mod.rs Cargo.toml Cargo.lock
git commit -m "feat(core): healthcheck stage — podman health poll or is-active"
```

---

## G4a completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] `Route` lives on `WorkloadSpec`; a routed spec without a `health_cmd` is rejected by `validate()`
- [ ] `%` is escaped in rendered `Environment=`/`Exec=` values (goldens unaffected)
- [ ] `DeployError`/`DeployOutcome`/`Ctx` exist and `thiserror` is now used (resolves that known-gap)
- [ ] `apply` restarts a changed unit and starts a new/unchanged one — existing apply tests still green
- [ ] `healthcheck` polls podman health or falls back to `is-active`; the timeout path is tested fast
- [ ] Nothing sequences the stages yet — no `deploy::run`, no lock lifecycle, no CLI (all G4b)

## Not in G4a — this is G4b

The `deploy::run` driver that acquires the per-app lock, creates the deploy row, runs
Detect → Build → Secrets → Apply → Route → Healthcheck (persisting the stage and emitting an event
after each, setting `spec.image` from the build), releases the lock on every exit path, and — on a
stage failure — runs the compensation matrix backward to `RolledBack` (or `Failed`). Then
`kuadrat deploy <app> <path>` with the unified `resolve_spec` (a repo `kuadrat.json`, falling back
to the stored spec, with CLI flag overrides — all funnelling into one `WorkloadSpec`), and the
real-host deploy-and-rollback acceptance (which needs `sudo` for system units, so it is operator-run
like phase 1's).
