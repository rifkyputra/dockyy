# kuadrat Phase 2 · G4b — The Deploy Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the G1–G4a pieces into `deploy::run` — the state machine that takes a repo to a running service and rolls back on failure — and expose it as `kuadrat deploy <app> <path>`.

**Architecture:** `run` acquires the per-app lock, creates a deploy row, then runs Detect → Build → Secrets → Apply → Route → Healthcheck, persisting the stage and emitting an event after each. A stage failure triggers backward compensation to `RolledBack` (or `Failed` if compensation also fails). The lock is released on every exit path. The CLI resolves one `WorkloadSpec` from a repo `kuadrat.json`, the stored spec, or flag overrides — a single unified interface into `run`.

**Tech Stack:** Rust (edition 2021), anyhow, serde_json, existing seams and stage helpers from G1–G4a, clap.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.**
- **Every host command goes through the `Executor`/`FileSystem` seams** (store carve-out excepted). The CLI reading the operator's `kuadrat.json` off local disk with `std::fs` is allowed (it is not a host interaction — ADR-0002).
- **The lock is released on EVERY exit path** — success, rollback, hard failure, and the early return when the spec is invalid or the lock is already held.
- Available from G1–G4a (do not reimplement): `deploy::detect::detect(exec, fsys, repo) -> BuildPlan`; `deploy::build::build(exec, plan, slug) -> String`; `deploy::health::healthcheck(exec, spec)`; `secrets::ensure_all(exec, names)`; `workloads::apply::{apply(exec, fsys, paths, spec), remove(exec, fsys, paths, name)}`; `gateway::{apply_route(exec, fsys, paths, slug, route), remove_route(exec, fsys, paths, slug)}`; `deploy::{Ctx, DeployOutcome, DeployStatus, Stage}`; `events::{Event, EventStatus}`; store `create_deploy/advance_stage/finish_deploy/acquire_lock/release_lock/put_spec/current_spec/append_event`; `WorkloadSpec::{validate, slug}`.

---

### Task 1: the driver's forward path

**Files:**
- Create: `crates/core/src/deploy/run.rs`
- Modify: `crates/core/src/deploy/mod.rs`

**Interfaces:**
- Consumes: all the stage helpers and store methods above
- Produces:
  - `deploy::run(ctx: &Ctx<'_>, spec: WorkloadSpec, repo: &Path) -> Result<DeployOutcome>`

`run` validates the spec, reads the previous stored spec (for later rollback), creates a deploy row, acquires the per-app lock (rejecting a concurrent deploy), runs every stage in order — persisting the stage and emitting Started/Succeeded events — and on success stores the spec and returns `Done`. On a stage failure it returns `Failed` (Task 2 adds compensation → `RolledBack`). The lock is released on every path.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/deploy/run.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::fake::FakeExecutor;
    use crate::exec::CommandOutput;
    use crate::fs::fake::FakeFileSystem;
    use crate::spec::WorkloadSpec;
    use crate::store::Store;
    use crate::workloads::paths::Paths;
    use std::path::Path;
    use tempfile::tempdir;

    fn out(status: i32, stdout: &str, stderr: &str) -> CommandOutput {
        CommandOutput { status, stdout: stdout.into(), stderr: stderr.into() }
    }

    /// Script a `FakeExecutor` for a clean deploy of an app with no secrets,
    /// no route, and no health_cmd. `start_result` lets a test fail the Apply.
    fn script_clean(exec: &FakeExecutor, sha: &str, slug: &str, start_result: CommandOutput) {
        exec.expect_call("git", &["-C", "/repo", "rev-parse", "HEAD"], out(0, &format!("{sha}\n"), ""));
        exec.expect_call(
            "podman",
            &["build", "-t", &format!("localhost/kuadrat-{slug}:{sha}"), "-f", "/repo/Containerfile", "/repo"],
            out(0, "", ""),
        );
        exec.expect_call("podman", &["secret", "ls", "--format", "{{.Name}}"], out(0, "", ""));
        exec.expect_call("systemctl", &["daemon-reload"], out(0, "", ""));
        exec.expect_call("systemctl", &["start", &format!("kuadrat-{slug}")], start_result);
        exec.expect_call("systemctl", &["is-active", &format!("kuadrat-{slug}")], out(0, "active\n", ""));
    }

    fn fsys_with_repo() -> FakeFileSystem {
        let fsys = FakeFileSystem::new();
        fsys.insert("/repo/Containerfile", "FROM alpine\n");
        fsys
    }

    #[tokio::test]
    async fn a_clean_deploy_runs_every_stage_and_returns_done() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = fsys_with_repo();
        let exec = FakeExecutor::new();
        script_clean(&exec, "abc123", "web", out(0, "", ""));

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcome = run(&ctx, WorkloadSpec::new("web", "placeholder"), Path::new("/repo"))
            .await
            .expect("deploy");

        assert_eq!(outcome, DeployOutcome::Done { image: "localhost/kuadrat-web:abc123".into() });
        // The lock was released: a fresh acquire succeeds.
        assert!(store.acquire_lock("web", 999).unwrap());
        // The spec was stored.
        assert!(store.current_spec("web").unwrap().is_some());
    }

    #[tokio::test]
    async fn a_stage_failure_returns_failed_and_releases_the_lock() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = fsys_with_repo();
        let exec = FakeExecutor::new();
        // Apply's `start` fails; healthcheck is never reached.
        script_clean(&exec, "abc123", "web", out(1, "", "boom"));

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcome = run(&ctx, WorkloadSpec::new("web", "placeholder"), Path::new("/repo"))
            .await
            .expect("run returns a terminal outcome, not an error");

        match outcome {
            DeployOutcome::Failed { failed_at, .. } => assert_eq!(failed_at, Stage::Apply),
            other => panic!("expected Failed at Apply, got {other:?}"),
        }
        assert!(store.acquire_lock("web", 999).unwrap(), "lock must be released even on failure");
    }

    #[tokio::test]
    async fn a_concurrent_deploy_is_rejected_while_the_lock_is_held() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = fsys_with_repo();
        let exec = FakeExecutor::new(); // no stage runs, so nothing to script

        // Another deploy already holds the lock.
        let other = store.create_deploy("web").unwrap();
        store.acquire_lock("web", other).unwrap();

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let err = run(&ctx, WorkloadSpec::new("web", "placeholder"), Path::new("/repo"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already in progress"), "message: {err}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `mod run;` and `pub use run::run;` to `crates/core/src/deploy/mod.rs`. Then:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core 'deploy::run' 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function run`.

- [ ] **Step 3: Write the driver**

Prepend to `crates/core/src/deploy/run.rs`:

```rust
//! The deploy state machine. `run` sequences Detect → Build → Secrets → Apply →
//! Route → Healthcheck, persisting the stage and emitting an event after each,
//! under the per-app lock. G4b Task 2 adds the compensation matrix; until then a
//! stage failure returns `Failed`.

use std::path::Path;

use anyhow::{bail, Context as _, Result};

use crate::deploy::build::build;
use crate::deploy::detect::detect;
use crate::deploy::health::healthcheck;
use crate::deploy::{Ctx, DeployOutcome, DeployStatus, Stage};
use crate::events::{Event, EventStatus};
use crate::gateway::apply_route;
use crate::secrets::ensure_all;
use crate::spec::WorkloadSpec;
use crate::workloads::apply::apply;

/// Deploy `spec` from the repo at `repo`. Returns the terminal outcome
/// (`Done`/`RolledBack`/`Failed`); returns `Err` only when the deploy could not
/// begin (invalid spec, or another deploy already holds the lock).
pub async fn run(ctx: &Ctx<'_>, mut spec: WorkloadSpec, repo: &Path) -> Result<DeployOutcome> {
    spec.validate()?;
    let name = spec.name.clone();
    let slug = spec.slug();

    let previous = load_previous(ctx, &name)?;

    let deploy_id = ctx.store.create_deploy(&name)?;
    if !ctx.store.acquire_lock(&name, deploy_id)? {
        ctx.store.finish_deploy(
            deploy_id,
            DeployStatus::Failed,
            Some("another deploy is already in progress"),
        )?;
        bail!("another deploy of {name} is already in progress");
    }

    // Everything past the lock must release it, whatever happens.
    let result = run_stages(ctx, spec, repo, &slug, deploy_id, &previous).await;
    ctx.store.release_lock(&name)?;
    result
}

fn load_previous(ctx: &Ctx<'_>, name: &str) -> Result<Option<WorkloadSpec>> {
    match ctx.store.current_spec(name)? {
        Some(json) => Ok(Some(
            serde_json::from_str(&json).context("parsing the previously stored spec")?,
        )),
        None => Ok(None),
    }
}

async fn run_stages(
    ctx: &Ctx<'_>,
    mut spec: WorkloadSpec,
    repo: &Path,
    slug: &str,
    deploy_id: i64,
    previous: &Option<WorkloadSpec>,
) -> Result<DeployOutcome> {
    begin(ctx, deploy_id, Stage::Detect)?;
    let plan = match detect(ctx.exec, ctx.fsys, repo).await {
        Ok(plan) => {
            ok(ctx, deploy_id, Stage::Detect)?;
            plan
        }
        Err(e) => return fail(ctx, deploy_id, &spec, slug, previous, Stage::Detect, e).await,
    };

    begin(ctx, deploy_id, Stage::Build)?;
    let image = match build(ctx.exec, &plan, slug).await {
        Ok(image) => {
            ok(ctx, deploy_id, Stage::Build)?;
            image
        }
        Err(e) => return fail(ctx, deploy_id, &spec, slug, previous, Stage::Build, e).await,
    };
    spec.image = image.clone();

    begin(ctx, deploy_id, Stage::Secrets)?;
    if let Err(e) = ensure_all(ctx.exec, &spec.secrets).await {
        return fail(ctx, deploy_id, &spec, slug, previous, Stage::Secrets, e).await;
    }
    ok(ctx, deploy_id, Stage::Secrets)?;

    begin(ctx, deploy_id, Stage::Apply)?;
    if let Err(e) = apply(ctx.exec, ctx.fsys, ctx.paths, &spec).await {
        return fail(ctx, deploy_id, &spec, slug, previous, Stage::Apply, e).await;
    }
    ok(ctx, deploy_id, Stage::Apply)?;

    begin(ctx, deploy_id, Stage::Route)?;
    if let Some(route) = &spec.route {
        if let Err(e) = apply_route(ctx.exec, ctx.fsys, ctx.paths, slug, route).await {
            return fail(ctx, deploy_id, &spec, slug, previous, Stage::Route, e).await;
        }
    }
    ok(ctx, deploy_id, Stage::Route)?;

    begin(ctx, deploy_id, Stage::Healthcheck)?;
    if let Err(e) = healthcheck(ctx.exec, &spec).await {
        return fail(ctx, deploy_id, &spec, slug, previous, Stage::Healthcheck, e).await;
    }
    ok(ctx, deploy_id, Stage::Healthcheck)?;

    let json = serde_json::to_string(&spec).context("serializing the deployed spec")?;
    ctx.store.put_spec(&spec.name, slug, &json)?;
    ctx.store.finish_deploy(deploy_id, DeployStatus::Done, None)?;
    Ok(DeployOutcome::Done { image })
}

/// Advance the durable stage and emit a Started event.
fn begin(ctx: &Ctx<'_>, deploy_id: i64, stage: Stage) -> Result<()> {
    ctx.store.advance_stage(deploy_id, stage)?;
    ctx.store.append_event(&Event {
        deploy_id,
        stage,
        status: EventStatus::Started,
        detail: None,
    })?;
    Ok(())
}

/// Emit a Succeeded event for a stage.
fn ok(ctx: &Ctx<'_>, deploy_id: i64, stage: Stage) -> Result<()> {
    ctx.store.append_event(&Event {
        deploy_id,
        stage,
        status: EventStatus::Succeeded,
        detail: None,
    })?;
    Ok(())
}

/// Handle a stage failure. G4b Task 1: no compensation — record it and return
/// `Failed`. Task 2 replaces the body with the compensation matrix.
async fn fail(
    ctx: &Ctx<'_>,
    deploy_id: i64,
    _spec: &WorkloadSpec,
    _slug: &str,
    _previous: &Option<WorkloadSpec>,
    stage: Stage,
    err: anyhow::Error,
) -> Result<DeployOutcome> {
    let cause = format!("{err:#}");
    ctx.store.append_event(&Event {
        deploy_id,
        stage,
        status: EventStatus::Failed,
        detail: Some(cause.clone()),
    })?;
    ctx.store
        .finish_deploy(deploy_id, DeployStatus::Failed, Some(&cause))?;
    Ok(DeployOutcome::Failed { failed_at: stage, cause })
}
```

Note: `fail`'s `_spec`/`_slug`/`_previous` params are unused in Task 1 (prefixed `_` to avoid warnings); Task 2 uses them for compensation. Keep them in the signature so Task 2 only changes the body.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core 'deploy::run'
```
Expected: 3 tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/run.rs crates/core/src/deploy/mod.rs
git commit -m "feat(core): deploy driver — sequence the stages under the per-app lock"
```

---

### Task 2: the compensation matrix

**Files:**
- Modify: `crates/core/src/deploy/run.rs`

**Interfaces:**
- Consumes: `workloads::apply::{apply, remove}`, `gateway::{apply_route, remove_route}`
- Produces: `fail` now compensates backward — `RolledBack` when the undo succeeds, `Failed` when it also fails

The compensation table:

| Failed at | Undo |
|---|---|
| Detect, Build, Secrets | Nothing — the host was never touched, old version still serving |
| Apply | Restore the previous spec (re-apply it), or remove the unit if there was no previous |
| Route | Restore the previous route (or remove the new one), then unwind Apply |
| Healthcheck | Unwind Route, then unwind Apply |

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/core/src/deploy/run.rs`:

```rust
    #[tokio::test]
    async fn a_first_deploy_failing_at_apply_rolls_back_by_removing_the_unit() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = fsys_with_repo();
        let exec = FakeExecutor::new();
        // Apply writes the unit, daemon-reload ok, start FAILS.
        script_clean(&exec, "abc123", "web", out(1, "", "boom"));
        // Compensation: no previous spec → remove the unit we just wrote.
        exec.expect_call("systemctl", &["stop", "kuadrat-web"], out(0, "", ""));
        // (remove also runs a second daemon-reload — already scripted by script_clean)

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcome = run(&ctx, WorkloadSpec::new("web", "placeholder"), Path::new("/repo"))
            .await
            .expect("terminal outcome");

        match outcome {
            DeployOutcome::RolledBack { failed_at, .. } => assert_eq!(failed_at, Stage::Apply),
            other => panic!("expected RolledBack at Apply, got {other:?}"),
        }
        // The half-written unit was removed by compensation.
        let unit = paths.quadlet_dir.join("kuadrat-web.container");
        assert!(fsys.contents(&unit).is_none(), "the unit should have been removed");
        assert!(store.acquire_lock("web", 999).unwrap(), "lock released");
    }

    #[tokio::test]
    async fn a_failure_at_healthcheck_unwinds_route_then_apply() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = fsys_with_repo();
        let exec = FakeExecutor::new();

        // Full forward path succeeds until healthcheck. No route on the spec, so
        // Route runs no command; is-active reports NOT active → Healthcheck fails.
        exec.expect_call("git", &["-C", "/repo", "rev-parse", "HEAD"], out(0, "abc123\n", ""));
        exec.expect_call(
            "podman",
            &["build", "-t", "localhost/kuadrat-web:abc123", "-f", "/repo/Containerfile", "/repo"],
            out(0, "", ""),
        );
        exec.expect_call("podman", &["secret", "ls", "--format", "{{.Name}}"], out(0, "", ""));
        exec.expect_call("systemctl", &["daemon-reload"], out(0, "", ""));
        exec.expect_call("systemctl", &["start", "kuadrat-web"], out(0, "", ""));
        exec.expect_call("systemctl", &["is-active", "kuadrat-web"], out(3, "failed\n", ""));
        // Compensation for a Healthcheck failure: unwind_route then unwind_apply.
        // The spec has no route, so no fragment was written — `remove_route` sees
        // an absent (unowned) fragment and returns early WITHOUT reloading Caddy,
        // so there is NO `reload caddy` call to script. unwind_apply has no
        // previous spec, so it removes the unit: `stop` then `daemon-reload`
        // (daemon-reload is already scripted above).
        exec.expect_call("systemctl", &["stop", "kuadrat-web"], out(0, "", ""));

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcome = run(&ctx, WorkloadSpec::new("web", "placeholder"), Path::new("/repo"))
            .await
            .expect("terminal outcome");

        match outcome {
            DeployOutcome::RolledBack { failed_at, .. } => assert_eq!(failed_at, Stage::Healthcheck),
            other => panic!("expected RolledBack at Healthcheck, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core 'deploy::run' 2>&1 | grep -E 'FAILED|panicked'
```
Expected: FAIL — the current `fail` returns `Failed`, not `RolledBack`.

- [ ] **Step 3: Replace `fail`'s body with compensation, add the helpers**

In `crates/core/src/deploy/run.rs`, add imports for `remove` and `remove_route`:

```rust
use crate::gateway::{apply_route, remove_route};
use crate::workloads::apply::{apply, remove};
```

Replace the `fail` function body:

```rust
/// Handle a stage failure: compensate backward. `RolledBack` when the undo
/// succeeds; `Failed` when compensation also fails (host state is unknown).
async fn fail(
    ctx: &Ctx<'_>,
    deploy_id: i64,
    spec: &WorkloadSpec,
    slug: &str,
    previous: &Option<WorkloadSpec>,
    stage: Stage,
    err: anyhow::Error,
) -> Result<DeployOutcome> {
    let cause = format!("{err:#}");
    ctx.store.append_event(&Event {
        deploy_id,
        stage,
        status: EventStatus::Failed,
        detail: Some(cause.clone()),
    })?;

    match compensate(ctx, &spec.name, slug, previous, stage).await {
        Ok(()) => {
            ctx.store
                .finish_deploy(deploy_id, DeployStatus::RolledBack, Some(&cause))?;
            Ok(DeployOutcome::RolledBack { failed_at: stage, cause })
        }
        Err(comp) => {
            let combined = format!("{cause}; compensation also failed: {comp:#}");
            ctx.store
                .finish_deploy(deploy_id, DeployStatus::Failed, Some(&combined))?;
            Ok(DeployOutcome::Failed { failed_at: stage, cause: combined })
        }
    }
}

/// Undo the host changes made before `failed_at`, walking backward.
async fn compensate(
    ctx: &Ctx<'_>,
    name: &str,
    slug: &str,
    previous: &Option<WorkloadSpec>,
    failed_at: Stage,
) -> Result<()> {
    match failed_at {
        // The host was never touched — the old version is still serving.
        Stage::Detect | Stage::Build | Stage::Secrets => Ok(()),
        Stage::Apply => unwind_apply(ctx, name, previous).await,
        Stage::Route | Stage::Healthcheck => {
            unwind_route(ctx, slug, previous).await?;
            unwind_apply(ctx, name, previous).await
        }
    }
}

/// Restore the previous unit (re-apply the previous spec), or remove the unit
/// this deploy wrote when there was no previous.
async fn unwind_apply(ctx: &Ctx<'_>, name: &str, previous: &Option<WorkloadSpec>) -> Result<()> {
    match previous {
        Some(prev) => apply(ctx.exec, ctx.fsys, ctx.paths, prev).await,
        None => remove(ctx.exec, ctx.fsys, ctx.paths, name).await,
    }
}

/// Restore the previous route, or remove the route this deploy wrote when there
/// was no previous route.
async fn unwind_route(ctx: &Ctx<'_>, slug: &str, previous: &Option<WorkloadSpec>) -> Result<()> {
    match previous.as_ref().and_then(|p| p.route.as_ref()) {
        Some(prev_route) => apply_route(ctx.exec, ctx.fsys, ctx.paths, slug, prev_route).await,
        None => remove_route(ctx.exec, ctx.fsys, ctx.paths, slug).await,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core 'deploy::run'
```
Expected: all run tests PASS.

**One Task-1 test is now superseded — DELETE it.** `a_stage_failure_returns_failed_and_releases_the_lock` asserted the pre-compensation `Failed`-at-Apply behavior, and it used `script_clean` without scripting the `systemctl stop` that compensation's `remove` now needs — so it can't simply be re-asserted. The new `a_first_deploy_failing_at_apply_rolls_back_by_removing_the_unit` test covers the exact same scenario correctly (Apply fails, no previous → rollback removes the unit → `RolledBack`), including the lock-released assertion. **Delete `a_stage_failure_returns_failed_and_releases_the_lock` entirely** — it is not a weakening, it is a Task-1 intermediate replaced by the correct rollback test. Do not weaken any other assertion.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean. `spec`/`slug`/`previous` in `fail` are now used (no `_` prefix needed — rename them).

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/run.rs
git commit -m "feat(core): compensation matrix — a failed deploy rolls back"
```

---

### Task 3: `kuadrat deploy` and the unified `resolve_spec`

**Files:**
- Create: `crates/cli/src/resolve.rs`
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Consumes: `kuadrat_core::{deploy::run, deploy::Ctx, spec::{WorkloadSpec, Route}, store::Store, exec::local::LocalExecutor, fs::local::LocalFileSystem, workloads::paths::Paths}`
- Produces:
  - `resolve::resolve_spec(app: &str, repo: &Path, store: &Store, route_override: Option<Route>) -> Result<WorkloadSpec>` — one `WorkloadSpec` from a repo `kuadrat.json`, else the stored spec, with the name forced to `app` and an optional route override
  - `kuadrat deploy <app> <path> [--route domain:port]`

`resolve_spec` is the single unified interface into the machine: the repo file, the stored spec, and CLI flags all funnel into one `WorkloadSpec` that `run` consumes.

- [ ] **Step 1: Write the failing test**

Create `crates/cli/src/resolve.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use kuadrat_core::spec::{Route, WorkloadSpec};
    use kuadrat_core::store::Store;
    use tempfile::tempdir;

    #[test]
    fn a_repo_kuadrat_json_is_the_primary_source() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("kuadrat.json"),
            r#"{"name":"ignored","image":"","command":null,"env":[],"ports":["3000:3000"],"volumes":[],"secrets":[],"memory_max":null,"health_cmd":null,"restart_policy":"Always","route":null}"#,
        )
        .unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let spec = resolve_spec("web", dir.path(), &store, None).unwrap();
        assert_eq!(spec.name, "web"); // name forced to the app arg
        assert_eq!(spec.ports, vec!["3000:3000".to_string()]);
    }

    #[test]
    fn the_stored_spec_is_the_fallback_when_no_file() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let mut prior = WorkloadSpec::new("web", "old");
        prior.ports = vec!["8080:8080".into()];
        store.put_spec("web", "web", &serde_json::to_string(&prior).unwrap()).unwrap();

        let spec = resolve_spec("web", dir.path(), &store, None).unwrap();
        assert_eq!(spec.ports, vec!["8080:8080".to_string()]);
    }

    #[test]
    fn no_file_and_no_stored_spec_is_an_error() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let err = resolve_spec("web", dir.path(), &store, None).unwrap_err();
        assert!(err.to_string().contains("kuadrat.json"), "message: {err}");
    }

    #[test]
    fn a_route_override_wins() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("kuadrat.json"),
            r#"{"name":"web","image":"","command":null,"env":[],"ports":[],"volumes":[],"secrets":[],"memory_max":null,"health_cmd":"true","restart_policy":"Always","route":null}"#,
        )
        .unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let route = Route { domain: "example.com".into(), port: 3000 };
        let spec = resolve_spec("web", dir.path(), &store, Some(route.clone())).unwrap();
        assert_eq!(spec.route, Some(route));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `mod resolve;` to `crates/cli/src/main.rs`. Then:
```bash
cargo test -p kuadrat resolve 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function resolve_spec`.

- [ ] **Step 3: Write `resolve_spec`**

Prepend to `crates/cli/src/resolve.rs`:

```rust
use std::path::Path;

use anyhow::{Context, Result};
use kuadrat_core::spec::{Route, WorkloadSpec};
use kuadrat_core::store::Store;

/// Resolve one `WorkloadSpec` for `app` from, in order: a `kuadrat.json` in the
/// repo, else the spec stored from a prior deploy. The name is forced to `app`,
/// and `route_override` (a CLI flag) replaces any route in the resolved spec.
pub fn resolve_spec(
    app: &str,
    repo: &Path,
    store: &Store,
    route_override: Option<Route>,
) -> Result<WorkloadSpec> {
    let file = repo.join("kuadrat.json");
    let mut spec: WorkloadSpec = if file.exists() {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", file.display()))?
    } else if let Some(json) = store.current_spec(app)? {
        serde_json::from_str(&json).context("parsing the stored spec")?
    } else {
        anyhow::bail!(
            "no spec for {app}: add a kuadrat.json to {} or deploy it once with one",
            repo.display()
        );
    };

    spec.name = app.to_string();
    if let Some(route) = route_override {
        spec.route = Some(route);
    }
    Ok(spec)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat resolve
```
Expected: 4 tests PASS.

- [ ] **Step 5: Wire the `deploy` subcommand**

In `crates/cli/src/main.rs`, add a `Deploy` variant to the `Command` enum:

```rust
    /// Build and deploy an app from a local repo
    Deploy {
        app: String,
        path: std::path::PathBuf,
        /// Route this app: domain:port (e.g. example.com:3000)
        #[arg(long)]
        route: Option<String>,
    },
```

And the match arm (reuse the existing `exec`; open a `Store` at the resolved db path; build a `Ctx`):

```rust
        Command::Deploy { app, path, route } => {
            use kuadrat_core::deploy::{run, Ctx, DeployOutcome};
            use kuadrat_core::fs::local::LocalFileSystem;
            use kuadrat_core::spec::Route;
            use kuadrat_core::store::Store;

            let route_override = match route {
                Some(s) => {
                    let (domain, port) = s
                        .rsplit_once(':')
                        .context("--route must be domain:port")?;
                    Some(Route {
                        domain: domain.to_string(),
                        port: port.parse().context("--route port must be a number")?,
                    })
                }
                None => None,
            };

            let store = Store::open(&paths.db_path)?;
            let spec = resolve::resolve_spec(&app, &path, &store, route_override)?;
            let fsys = LocalFileSystem;
            let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
            let outcome = run(&ctx, spec, &path).await?;
            println!("{outcome:?}");
            // A rolled-back or failed deploy exits non-zero (CI-friendly); only
            // `Done` is success.
            if !matches!(outcome, DeployOutcome::Done { .. }) {
                std::process::exit(1);
            }
        }
```

`paths` is already in scope in `main` (built from `--root` or `Paths::default()`), and `use anyhow::Context;` is already imported.

- [ ] **Step 6: Verify the CLI compiles and the gate is clean**

```bash
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: builds clean, zero warnings, every test-result line `0 failed`.

- [ ] **Step 7: Commit**

```bash
git add crates/cli/src/resolve.rs crates/cli/src/main.rs
git commit -m "feat(cli): kuadrat deploy with a unified resolve_spec"
```

---

### Task 4: real-host deploy-and-rollback acceptance

**Files:**
- Create: `scripts/deploy-acceptance.sh`

**Interfaces:**
- Consumes: the `kuadrat` binary, real podman + systemd
- Produces: a script that deploys a fixture repo, verifies it runs, deploys a broken commit, and verifies rollback left the working version serving

This deploy touches **system** Quadlet units (`/etc/containers/systemd`) and `systemctl daemon-reload`, so it needs **root** — the subagent writes and syntax-checks it, but the full run is operator-executed with `sudo`, like phase 1's acceptance. There is no route in the fixture (a live route needs Caddy + a public domain — that is G5), so this proves the deploy loop and rollback, not TLS.

- [ ] **Step 1: Write the acceptance script**

Create `scripts/deploy-acceptance.sh`:

```bash
#!/usr/bin/env bash
# kuadrat G4b deploy acceptance. Needs root (system Quadlet units):
#   sudo bash scripts/deploy-acceptance.sh
# Build the binary first (as your normal user):
#   PATH=$HOME/.cargo/bin:$PATH cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
APP=g4bdemo
SLUG=g4bdemo
UNIT=kuadrat-${SLUG}
WORK=$(mktemp -d)

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }
cleanup() {
  "$BIN" remove "$APP" >/dev/null 2>&1
  rm -rf "$WORK"
  systemctl daemon-reload
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found — build it as your user first"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "FATAL: run as root (sudo) — system Quadlet units need it"; exit 1; }

echo "kuadrat G4b deploy acceptance"
echo "podman : $(podman --version 2>/dev/null || echo MISSING)"

# A fixture repo: a working app that stays up, plus a kuadrat.json.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
CMD ["sh", "-c", "echo v1 up; sleep 3600"]
EOF
cat > "$WORK/$APP/kuadrat.json" <<'EOF'
{"name":"g4bdemo","image":"","command":null,"env":[],"ports":[],"volumes":[],
 "secrets":[],"memory_max":"128M","health_cmd":null,"restart_policy":"Always","route":null}
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm v1

echo "== deploy v1"
OUT=$("$BIN" deploy "$APP" "$WORK/$APP" 2>&1); echo "$OUT"
echo "$OUT" | grep -q 'Done' && ok "deploy v1 -> Done" || bad "deploy v1 did not reach Done"
systemctl is-active --quiet "$UNIT" && ok "v1 unit active" || bad "v1 unit not active"

echo "== deploy a broken commit (bad Containerfile) and expect rollback"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
RUN exit 1
EOF
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -aqm broken
OUT2=$("$BIN" deploy "$APP" "$WORK/$APP" 2>&1); echo "$OUT2"
echo "$OUT2" | grep -qE 'RolledBack|Failed' && ok "broken deploy did not report Done" || bad "broken deploy unexpectedly reported Done"
# The v1 unit must still be present and active after the failed deploy.
systemctl is-active --quiet "$UNIT" && ok "v1 still active after the failed deploy" || bad "v1 was lost by the failed deploy"

echo "== remove"
"$BIN" remove "$APP" >/dev/null 2>&1
systemctl is-active --quiet "$UNIT" && bad "unit still active after remove" || ok "unit stopped after remove"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  G4B DEPLOY ACCEPTANCE: PASS" || echo "  G4B DEPLOY ACCEPTANCE: FAIL"
exit $fail
```

Make it executable: `chmod +x scripts/deploy-acceptance.sh`.

- [ ] **Step 2: Syntax-check the script (no root needed)**

```bash
bash -n scripts/deploy-acceptance.sh && echo "syntax OK"
```
Expected: `syntax OK`. Do NOT run the script itself — it needs root, which the subagent does not have. The operator runs it (see the completion note).

- [ ] **Step 3: Build the release binary and run the whole suite**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: builds clean, zero warnings, every test-result line `0 failed`.

- [ ] **Step 4: Commit**

```bash
git add scripts/deploy-acceptance.sh
git commit -m "test: add the G4b real-host deploy-and-rollback acceptance script"
```

---

## G4b completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] `deploy::run` sequences all six stages, persists the stage and emits an event after each, and releases the lock on every exit path
- [ ] A stage failure compensates backward — `RolledBack` on success, `Failed` when compensation also fails
- [ ] `kuadrat deploy <app> <path>` resolves one spec (file → stored → flag override) and runs it
- [ ] `scripts/deploy-acceptance.sh` exists and syntax-checks; it is operator-run with `sudo`

## Operator step (needs root — hand this to the human)

The subagent cannot run the deploy acceptance (system Quadlet units need root). After G4b merges, run it as the operator:

```bash
cd ~/devbox/kuadrat && PATH=$HOME/.cargo/bin:$PATH cargo build --release
sudo bash scripts/deploy-acceptance.sh
```

Expected: `G4B DEPLOY ACCEPTANCE: PASS` — v1 deploys to `Done`, a broken commit rolls back, and v1 stays active throughout.

## Not in G4b — this is G5

Crash reconciliation on daemon start (finish or roll back an `in_progress` deploy, release its lock) and the extended acceptance that kills a deploy mid-flight. Also deferred there or beyond: a live route over TLS in the acceptance (needs Caddy + a public domain), the pull/reconcile layer, and the web UI/MCP surfaces (phases 3 and 4).
