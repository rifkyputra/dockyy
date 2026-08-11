# kuadrat Phase 2 · G5 — Reconciliation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recover cleanly from a deploy that crashed mid-flight — `deploy::reconcile` finds every `in_progress` deploy, rolls it back to the last-good state, and releases its lock; exposed as `kuadrat reconcile`.

**Architecture:** A crash leaves a coherent triple in the store (an `in_progress` deploy row, its durable stage, a held lock) that G4b was built to produce. `reconcile` reads `in_progress_deploys()` and, for each, runs the SAME backward compensation the driver uses — from the durable stage — then finishes the row and releases the lock. It reuses `compensate`/`load_previous` from the driver, so recovery and rollback are one code path.

**Tech Stack:** Rust (edition 2021), anyhow, existing driver + store from G1–G4b, clap. The acceptance uses the `sqlite3` CLI to simulate a crash.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.** Every host command via the `Executor`/`FileSystem` seams (store carve-out excepted).
- **The lock is released for EVERY reconciled deploy** — both when its rollback succeeds (`RolledBack`) and when the rollback itself fails (`Failed`). A reconciled deploy is terminally finished; leaving its lock held would re-brick the app.
- Available (G1–G4b): `store::in_progress_deploys() -> Vec<DeployRow>` (`DeployRow { id, app, stage, status, detail }`); the private-in-`run.rs` helpers `compensate(ctx, name, slug, previous, failed_at)` and `load_previous(ctx, name)`; `store::{finish_deploy, release_lock}`; `deploy::{Ctx, DeployOutcome, DeployStatus, Stage}`; `spec::slug`.

---

### Task 1: `deploy::reconcile`

**Files:**
- Modify: `crates/core/src/deploy/run.rs`
- Modify: `crates/core/src/deploy/mod.rs`

**Interfaces:**
- Consumes: `store::in_progress_deploys`, the private `compensate`/`load_previous`, `store::{finish_deploy, release_lock}`
- Produces: `deploy::reconcile(ctx: &Ctx<'_>) -> Result<Vec<DeployOutcome>>`

`reconcile` lives in `run.rs` so it reuses the driver's private `compensate`/`load_previous` directly. For each `in_progress` deploy it loads the last-good spec, runs `compensate` from the deploy's durable stage, finishes the row (`RolledBack`, or `Failed` if compensation itself fails), and releases the lock. It never rebuilds — it restores the last-good unit.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/deploy/run.rs` (it already has `out`, `script_clean`, `fsys_with_repo` helpers and imports `FakeExecutor`, `FakeFileSystem`, `Store`, `Paths`, `WorkloadSpec`, `Path`, `tempdir`):

```rust
    #[tokio::test]
    async fn reconcile_is_a_noop_when_nothing_is_in_progress() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = FakeFileSystem::new();
        let exec = FakeExecutor::new();
        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        assert!(reconcile(&ctx).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn reconcile_rolls_back_a_crash_at_detect_with_no_host_changes() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = FakeFileSystem::new();
        // A crash left an in_progress deploy stuck at Detect, lock held.
        let id = store.create_deploy("web").unwrap();
        store.advance_stage(id, Stage::Detect).unwrap();
        store.acquire_lock("web", id).unwrap();

        let exec = FakeExecutor::new(); // Detect touched nothing, so no host calls
        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcomes = reconcile(&ctx).await.unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], DeployOutcome::RolledBack { failed_at: Stage::Detect, .. }));
        assert!(store.in_progress_deploys().unwrap().is_empty(), "row finished");
        assert!(store.acquire_lock("web", 999).unwrap(), "lock released");
    }

    #[tokio::test]
    async fn reconcile_restores_the_previous_spec_after_a_crash_at_apply() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = FakeFileSystem::new();
        // A prior successful deploy stored a spec.
        let prev = WorkloadSpec::new("web", "old:1");
        store.put_spec("web", "web", &serde_json::to_string(&prev).unwrap()).unwrap();
        // The next deploy crashed at Apply.
        let id = store.create_deploy("web").unwrap();
        store.advance_stage(id, Stage::Apply).unwrap();
        store.acquire_lock("web", id).unwrap();

        // Reconcile re-applies the previous spec: daemon-reload + start.
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["daemon-reload"], out(0, "", ""));
        exec.expect_call("systemctl", &["start", "kuadrat-web"], out(0, "", ""));

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcomes = reconcile(&ctx).await.unwrap();

        assert!(matches!(outcomes[0], DeployOutcome::RolledBack { failed_at: Stage::Apply, .. }));
        assert!(store.in_progress_deploys().unwrap().is_empty());
        assert!(store.acquire_lock("web", 999).unwrap());
    }

    #[tokio::test]
    async fn reconcile_removes_a_partial_unit_from_a_crashed_first_deploy() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let paths = Paths::rooted(dir.path());
        let fsys = FakeFileSystem::new();
        // The crashed first deploy wrote a marker-owned unit before dying at Apply.
        let unit = paths.quadlet_dir.join("kuadrat-web.container");
        fsys.insert(&unit, "# kuadrat-managed: true\n[Container]\nImage=x\n");
        let id = store.create_deploy("web").unwrap();
        store.advance_stage(id, Stage::Apply).unwrap();
        store.acquire_lock("web", id).unwrap();
        // No previous spec → reconcile removes the unit: stop + daemon-reload.
        let exec = FakeExecutor::new();
        exec.expect_call("systemctl", &["stop", "kuadrat-web"], out(0, "", ""));
        exec.expect_call("systemctl", &["daemon-reload"], out(0, "", ""));

        let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
        let outcomes = reconcile(&ctx).await.unwrap();

        assert!(matches!(outcomes[0], DeployOutcome::RolledBack { .. }));
        assert!(fsys.contents(&unit).is_none(), "partial unit removed");
        assert!(store.acquire_lock("web", 999).unwrap());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub use run::reconcile;` to `crates/core/src/deploy/mod.rs` (next to `pub use run::run;`). Then:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core reconcile 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find function reconcile`.

- [ ] **Step 3: Write `reconcile`**

In `crates/core/src/deploy/run.rs`, ensure `slug` is imported (change `use crate::spec::WorkloadSpec;` to `use crate::spec::{slug, WorkloadSpec};`). Add the function (near `run`, at the top level of the module — NOT inside another fn):

```rust
/// Recover from crashed deploys. For every deploy still `in_progress` (a crash
/// left it un-finished with its lock held), roll it back to the last-good state
/// using the same compensation the driver uses, then release its lock. Returns
/// one outcome per reconciled deploy. Safe to call on every startup — a no-op
/// when nothing is in progress.
pub async fn reconcile(ctx: &Ctx<'_>) -> Result<Vec<DeployOutcome>> {
    let mut outcomes = Vec::new();

    for row in ctx.store.in_progress_deploys()? {
        let previous = load_previous(ctx, &row.app)?;
        let app_slug = slug(&row.app);

        let outcome = match compensate(ctx, &row.app, &app_slug, &previous, row.stage).await {
            Ok(()) => {
                let cause = format!(
                    "reconciled after restart (was in progress at {:?})",
                    row.stage
                );
                ctx.store
                    .finish_deploy(row.id, DeployStatus::RolledBack, Some(&cause))?;
                DeployOutcome::RolledBack { failed_at: row.stage, cause }
            }
            Err(e) => {
                let cause = format!("reconcile compensation failed: {e:#}");
                ctx.store
                    .finish_deploy(row.id, DeployStatus::Failed, Some(&cause))?;
                DeployOutcome::Failed { failed_at: row.stage, cause }
            }
        };

        // The deploy is terminally finished either way — release its lock.
        ctx.store.release_lock(&row.app)?;
        outcomes.push(outcome);
    }

    Ok(outcomes)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core reconcile
```
Expected: 4 tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean. (`slug` is now used by `reconcile`; the merged import has no unused half.)

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy/run.rs crates/core/src/deploy/mod.rs
git commit -m "feat(core): reconcile — roll back crashed in-progress deploys on startup"
```

---

### Task 2: `kuadrat reconcile` + the reconcile acceptance

**Files:**
- Modify: `crates/cli/src/main.rs`
- Create: `scripts/reconcile-acceptance.sh`

**Interfaces:**
- Consumes: `kuadrat_core::deploy::{reconcile, Ctx}`, `store::Store`, the local executor/filesystem
- Produces: `kuadrat reconcile` — runs reconciliation and prints what it recovered

`kuadrat reconcile` is what an operator (and, later, the daemon on startup) runs after a crash. The acceptance simulates a crash by injecting an `in_progress` deploy row and a held lock with `sqlite3`, then proves reconcile clears it and unblocks the app.

- [ ] **Step 1: Add the `reconcile` subcommand**

In `crates/cli/src/main.rs`, add a variant to the `Command` enum:

```rust
    /// Recover from crashed deploys: roll back anything left in progress
    Reconcile,
```

And the match arm (reuse the in-scope `exec`/`fsys`; open a `Store` at the resolved db path):

```rust
        Command::Reconcile => {
            use kuadrat_core::deploy::{reconcile, Ctx};
            use kuadrat_core::store::Store;

            let store = Store::open(&paths.db_path)?;
            let ctx = Ctx { exec: &exec, fsys: &fsys, store: &store, paths: &paths };
            let outcomes = reconcile(&ctx).await?;
            if outcomes.is_empty() {
                println!("nothing to reconcile");
            } else {
                for outcome in &outcomes {
                    println!("{outcome:?}");
                }
            }
        }
```

`paths`, `exec`, and `fsys` are already in scope in `main` (the deploy/apply arms use them).

- [ ] **Step 2: Verify the CLI compiles and the gate is clean**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: builds clean, zero warnings, every test-result line `0 failed`.

- [ ] **Step 3: Write the acceptance script**

Create `scripts/reconcile-acceptance.sh`. It needs **root** (system Quadlet units) and the **`sqlite3` CLI** (to inject the simulated crash):

```bash
#!/usr/bin/env bash
# kuadrat G5 reconcile acceptance. Needs root AND the sqlite3 CLI:
#   sudo bash scripts/reconcile-acceptance.sh
# Build first (as your user): PATH=$HOME/.cargo/bin:$PATH cargo build --release

set -uo pipefail

BIN=/home/kyy/devbox/kuadrat/target/release/kuadrat
DB=/var/lib/kuadrat/kuadrat.db
APP=g5demo
UNIT=kuadrat-${APP}
WORK=$(mktemp -d)

pass=0; fail=0
ok()  { echo "  PASS  $1"; pass=$((pass+1)); }
bad() { echo "  FAIL  $1"; fail=$((fail+1)); }
cleanup() { "$BIN" remove "$APP" >/dev/null 2>&1; rm -rf "$WORK"; systemctl daemon-reload; }
trap cleanup EXIT

[ -x "$BIN" ] || { echo "FATAL: $BIN not found — build it as your user first"; exit 1; }
[ "$(id -u)" -eq 0 ] || { echo "FATAL: run as root (sudo)"; exit 1; }
command -v sqlite3 >/dev/null || { echo "FATAL: this acceptance needs the sqlite3 CLI (apt install sqlite3)"; exit 1; }

echo "kuadrat G5 reconcile acceptance"

# A working fixture app.
mkdir -p "$WORK/$APP"
cat > "$WORK/$APP/Containerfile" <<'EOF'
FROM docker.io/library/alpine:3
CMD ["sh", "-c", "echo up; sleep 3600"]
EOF
cat > "$WORK/$APP/kuadrat.json" <<'EOF'
{"name":"g5demo","image":"","command":null,"env":[],"ports":[],"volumes":[],
 "secrets":[],"memory_max":"128M","health_cmd":null,"restart_policy":"Always","route":null}
EOF
git -C "$WORK/$APP" init -q
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t add -A
git -C "$WORK/$APP" -c user.email=t@t -c user.name=t commit -qm v1

echo "== deploy g5demo"
"$BIN" deploy "$APP" "$WORK/$APP" 2>&1 | grep -q 'Done' && ok "deploy -> Done" || bad "deploy did not reach Done"

echo "== simulate a crash: inject an in_progress deploy + a held lock"
sqlite3 "$DB" "INSERT INTO deploys (app, stage, status) VALUES ('$APP', 'apply', 'in_progress');"
sqlite3 "$DB" "INSERT INTO locks (app, deploy_id) VALUES ('$APP', (SELECT max(id) FROM deploys));"
[ "$(sqlite3 "$DB" "SELECT count(*) FROM deploys WHERE status='in_progress';")" = "1" ] \
  && ok "injected a stuck in_progress deploy" || bad "injection failed"

echo "== the stuck lock blocks a new deploy"
"$BIN" deploy "$APP" "$WORK/$APP" 2>&1 | grep -q 'already in progress' \
  && ok "the held lock blocks a deploy" || bad "a deploy was NOT blocked by the stuck lock"

echo "== reconcile"
"$BIN" reconcile 2>&1 | grep -qE 'RolledBack|Failed' && ok "reconcile reported a recovery" || bad "reconcile recovered nothing"
[ "$(sqlite3 "$DB" "SELECT count(*) FROM deploys WHERE status='in_progress';")" = "0" ] \
  && ok "no in_progress deploys after reconcile" || bad "an in_progress deploy remained"

echo "== the app is unblocked and still running"
"$BIN" deploy "$APP" "$WORK/$APP" 2>&1 | grep -q 'Done' && ok "deploy works again after reconcile" || bad "deploy still blocked after reconcile"
systemctl is-active --quiet "$UNIT" && ok "g5demo still active" || bad "g5demo not active"

echo "== RESULT"
echo "  passed: $pass    failed: $fail"
[ $fail -eq 0 ] && echo "  G5 RECONCILE ACCEPTANCE: PASS" || echo "  G5 RECONCILE ACCEPTANCE: FAIL"
exit $fail
```

Make it executable: `chmod +x scripts/reconcile-acceptance.sh`.

- [ ] **Step 4: Syntax-check the script (no root needed) and run the gate**

```bash
bash -n scripts/reconcile-acceptance.sh && echo "syntax OK"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: `syntax OK`; builds clean; zero warnings; every test-result line `0 failed`. Do NOT run the script — it needs root (the operator runs it).

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/main.rs scripts/reconcile-acceptance.sh
git commit -m "feat(cli): kuadrat reconcile; add the G5 reconcile acceptance script"
```

---

## G5 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] `reconcile` rolls back every `in_progress` deploy from its durable stage, using the driver's own compensation
- [ ] Every reconciled deploy is finished (`RolledBack`/`Failed`) and its lock released
- [ ] `reconcile` is a no-op when nothing is in progress
- [ ] `kuadrat reconcile` exists; `scripts/reconcile-acceptance.sh` syntax-checks
- [ ] Reconcile reuses `compensate`/`load_previous` — recovery and rollback are ONE code path, not two

## Operator step (needs root + sqlite3 — hand this to the human)

After G5 merges, run the two acceptances as the operator:

```bash
cd ~/devbox/kuadrat && PATH=$HOME/.cargo/bin:$PATH cargo build --release
sudo bash scripts/deploy-acceptance.sh      # G4b: deploy + rollback
sudo bash scripts/reconcile-acceptance.sh   # G5: crash recovery (needs sqlite3)
```

Expected: both end `... ACCEPTANCE: PASS`. Together they prove the whole deploy loop end to end on a real host.

## Phase 2 complete after G5

With G5, phase 2's design is fully implemented: the store, detect/build, gateway/secrets, the deploy machine with compensation, and crash reconciliation. Deferred beyond phase 2 (phases 3–4 and later): a live route over TLS in an acceptance (needs Caddy + a public domain), the pull/reconcile *loop* (a timer that calls `deploy`), the web UI, and the MCP surface. The `known-gaps.md` items (the `release_lock`-drops-outcome minor, kuadrat.json ergonomics with `serde(default)`, the M1/M2 API tidy-ups) remain as recorded.
