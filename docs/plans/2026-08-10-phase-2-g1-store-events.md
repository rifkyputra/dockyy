# kuadrat Phase 2 · G1 — Store + Events Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the SQLite-backed `store` module and the persisted value types it reads and writes, so a deploy can be recorded, advanced through stages, locked against, and have events appended — with everything durable across a process restart.

**Architecture:** A synchronous `Store` wrapping one `rusqlite::Connection` behind a `Mutex`, exposed to the async engine as `&Store`. Deploy stage and status are stored as TEXT so the database is readable with the `sqlite3` CLI. The store is kuadrat's own state, so it opens its SQLite file directly rather than through the `FileSystem` seam — the sanctioned carve-out in ADR-0002.

**Tech Stack:** Rust (edition 2021), rusqlite (bundled SQLite), anyhow, thiserror, serde. Existing: tokio, async-trait.

## Global Constraints

- **`make check && make test` must pass with ZERO warnings.** `make check` = `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`. Run `cargo fmt` before every commit.
- **The Rust toolchain is NOT on the default PATH.** Every shell must first `export PATH="$HOME/.cargo/bin:$PATH"`. Verify with `cargo --version`; if missing, report BLOCKED.
- **`kuadrat-core` never opens a socket and never takes a `host` parameter.** The store is not an exception — no `host` argument appears anywhere.
- **The store carve-out is explicit and sanctioned.** `store` may call `rusqlite::Connection::open` and `std::fs::create_dir_all` directly. This is NOT a `FileSystem`-seam violation: the DB is kuadrat's own state, not a side effect on a managed host. It is the only place in the crate allowed to touch the filesystem outside `fs::local`. ADR-0002 is amended in Task 3 to record this.
- **Do not build, in G1** (later task groups): Detect, Build, Secrets, Apply, Route, Healthcheck, the state-machine driver, compensation, reconciliation logic, the gateway, `run_with_stdin`, or any `podman`/`systemctl` call. G1 is persistence and its value types only.
- **Secret values never touch the store.** Specs are stored as opaque JSON; the store never parses them and never logs their contents. (Secret *values* live in `podman secret`, added in G3.)
- Paths are injectable: no hardcoded `/var/lib/...` outside `Paths::default()`.

---

### Task 1: Add rusqlite and the store path

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Modify: `crates/core/Cargo.toml` (`[dependencies]`)
- Modify: `crates/core/src/workloads/paths.rs`

**Interfaces:**
- Consumes: `Paths` (phase 1)
- Produces:
  - workspace dep `rusqlite = { version = "0.32", features = ["bundled"] }`
  - `Paths { quadlet_dir: PathBuf, db_path: PathBuf }` — a new field
  - `Paths::default().db_path` → `/var/lib/kuadrat/kuadrat.db`
  - `Paths::rooted(root).db_path` → `<root>/lib/kuadrat/kuadrat.db`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/core/src/workloads/paths.rs`:

```rust
#[test]
fn db_path_default_is_under_var_lib() {
    let paths = Paths::default();
    assert_eq!(paths.db_path, std::path::PathBuf::from("/var/lib/kuadrat/kuadrat.db"));
}

#[test]
fn db_path_is_rerooted_for_tests() {
    let paths = Paths::rooted(std::path::Path::new("/tmp/kx"));
    assert_eq!(
        paths.db_path,
        std::path::PathBuf::from("/tmp/kx/lib/kuadrat/kuadrat.db")
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p kuadrat-core paths 2>&1 | grep -E 'error|db_path'
```
Expected: FAIL — `no field db_path on type Paths`.

- [ ] **Step 3: Add the dependency**

In the workspace root `Cargo.toml`, under `[workspace.dependencies]`, add:

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

In `crates/core/Cargo.toml`, under `[dependencies]`, add:

```toml
rusqlite.workspace = true
```

The `bundled` feature compiles SQLite from source, so no system `libsqlite3` is required — important for reproducibility across hosts.

- [ ] **Step 4: Add the `db_path` field**

In `crates/core/src/workloads/paths.rs`, extend the struct and both constructors:

```rust
#[derive(Debug, Clone)]
pub struct Paths {
    pub quadlet_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            quadlet_dir: PathBuf::from("/etc/containers/systemd"),
            db_path: PathBuf::from("/var/lib/kuadrat/kuadrat.db"),
        }
    }
}

impl Paths {
    /// All paths relative to `root` — for tests and dry runs.
    pub fn rooted(root: &Path) -> Self {
        Self {
            quadlet_dir: root.join("containers/systemd"),
            db_path: root.join("lib/kuadrat/kuadrat.db"),
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core paths
```
Expected: all paths tests PASS. The first `cargo build` here will compile bundled SQLite — it takes a minute; that is normal.

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/core/Cargo.toml crates/core/src/workloads/paths.rs
git commit -m "feat(core): add rusqlite dependency and the store path"
```

---

### Task 2: Persisted value types — Stage, DeployStatus, EventStatus, Event

**Files:**
- Create: `crates/core/src/deploy/mod.rs`
- Create: `crates/core/src/events/mod.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `deploy::Stage` — `Detect | Build | Secrets | Apply | Route | Healthcheck`, with `fn as_str(&self) -> &'static str` and `fn from_str(s: &str) -> Option<Stage>`
  - `deploy::DeployStatus` — `InProgress | Done | RolledBack | Failed`, same two methods
  - `events::EventStatus` — `Started | Succeeded | Failed`, same two methods
  - `events::Event { deploy_id: i64, stage: deploy::Stage, status: EventStatus, detail: Option<String> }`

These are the value types the store serializes. `deploy/mod.rs` holds only these enums in G1; the state machine (G4) fills in the rest of the module.

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/deploy/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_round_trips_through_its_string_form() {
        for stage in [
            Stage::Detect,
            Stage::Build,
            Stage::Secrets,
            Stage::Apply,
            Stage::Route,
            Stage::Healthcheck,
        ] {
            assert_eq!(Stage::from_str(stage.as_str()), Some(stage));
        }
    }

    #[test]
    fn stage_rejects_an_unknown_string() {
        assert_eq!(Stage::from_str("nonsense"), None);
    }

    #[test]
    fn deploy_status_round_trips() {
        for status in [
            DeployStatus::InProgress,
            DeployStatus::Done,
            DeployStatus::RolledBack,
            DeployStatus::Failed,
        ] {
            assert_eq!(DeployStatus::from_str(status.as_str()), Some(status));
        }
    }
}
```

Create `crates/core/src/events/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Stage;

    #[test]
    fn event_status_round_trips() {
        for status in [EventStatus::Started, EventStatus::Succeeded, EventStatus::Failed] {
            assert_eq!(EventStatus::from_str(status.as_str()), Some(status));
        }
    }

    #[test]
    fn an_event_carries_its_stage_and_detail() {
        let ev = Event {
            deploy_id: 7,
            stage: Stage::Build,
            status: EventStatus::Failed,
            detail: Some("image build failed".into()),
        };
        assert_eq!(ev.deploy_id, 7);
        assert_eq!(ev.stage, Stage::Build);
        assert_eq!(ev.status, EventStatus::Failed);
        assert_eq!(ev.detail.as_deref(), Some("image build failed"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add to `crates/core/src/lib.rs`, in alphabetical order among the existing `pub mod` lines:

```rust
pub mod deploy;
pub mod events;
```

Then:
```bash
cargo test -p kuadrat-core 'deploy::' 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find type Stage`.

- [ ] **Step 3: Write the deploy enums**

Prepend to `crates/core/src/deploy/mod.rs`:

```rust
//! Deploy value types. The state machine that uses them lands in G4.

/// A stage of the deploy loop. Stored as TEXT so the database is readable
/// with the `sqlite3` CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Detect,
    Build,
    Secrets,
    Apply,
    Route,
    Healthcheck,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Detect => "detect",
            Stage::Build => "build",
            Stage::Secrets => "secrets",
            Stage::Apply => "apply",
            Stage::Route => "route",
            Stage::Healthcheck => "healthcheck",
        }
    }

    // Inherent `from_str` returning Option, not the `FromStr` trait (which
    // returns Result). The store wants an Option to `.ok_or_else` on.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Stage> {
        match s {
            "detect" => Some(Stage::Detect),
            "build" => Some(Stage::Build),
            "secrets" => Some(Stage::Secrets),
            "apply" => Some(Stage::Apply),
            "route" => Some(Stage::Route),
            "healthcheck" => Some(Stage::Healthcheck),
            _ => None,
        }
    }
}

/// Terminal or in-flight status of a whole deploy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStatus {
    InProgress,
    Done,
    RolledBack,
    Failed,
}

impl DeployStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeployStatus::InProgress => "in_progress",
            DeployStatus::Done => "done",
            DeployStatus::RolledBack => "rolled_back",
            DeployStatus::Failed => "failed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<DeployStatus> {
        match s {
            "in_progress" => Some(DeployStatus::InProgress),
            "done" => Some(DeployStatus::Done),
            "rolled_back" => Some(DeployStatus::RolledBack),
            "failed" => Some(DeployStatus::Failed),
            _ => None,
        }
    }
}
```

- [ ] **Step 4: Write the events types**

Prepend to `crates/core/src/events/mod.rs`:

```rust
//! Typed deploy events. G1 defines the type and its status; the store persists
//! them (Task 7). Live emission (a subscriber channel) arrives with the daemon
//! in phase 3.

use crate::deploy::Stage;

/// What happened to a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    Started,
    Succeeded,
    Failed,
}

impl EventStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventStatus::Started => "started",
            EventStatus::Succeeded => "succeeded",
            EventStatus::Failed => "failed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<EventStatus> {
        match s {
            "started" => Some(EventStatus::Started),
            "succeeded" => Some(EventStatus::Succeeded),
            "failed" => Some(EventStatus::Failed),
            _ => None,
        }
    }
}

/// One durable event in a deploy's timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub deploy_id: i64,
    pub stage: Stage,
    pub status: EventStatus,
    pub detail: Option<String>,
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core deploy:: events::
```
Expected: 5 tests PASS.

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean. The `#[allow(clippy::should_implement_trait)]` on each `from_str` is deliberate and already in the code above — the inherent `Option`-returning method is what the store calls; the `FromStr` trait would force `Result`.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/deploy/mod.rs crates/core/src/events/mod.rs crates/core/src/lib.rs
git commit -m "feat(core): add Stage, DeployStatus, EventStatus, and Event value types"
```

---

### Task 3: Store open and schema

**Files:**
- Create: `crates/core/src/store/mod.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `docs/adr/0002-transport-agnostic-core.md`

**Interfaces:**
- Consumes: nothing (opens a path directly)
- Produces:
  - `store::Store` — holds `Mutex<rusqlite::Connection>`; `Send + Sync`
  - `Store::open(path: &Path) -> anyhow::Result<Store>` — creates parent dirs, opens, runs the schema; idempotent

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/store/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_the_database_and_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("lib/kuadrat/kuadrat.db");

        let store = Store::open(&db).expect("open");
        assert!(db.exists(), "db file should be created");
        drop(store);

        // Opening again over the same file must not error (IF NOT EXISTS).
        Store::open(&db).expect("reopen");
    }

    #[test]
    fn open_creates_the_expected_tables() {
        let dir = tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("k.db")).expect("open");
        let conn = store.conn.lock().expect("lock");
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type='table' AND name IN ('apps','deploys','locks','events')",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 4, "all four tables should exist");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `pub mod store;` to `crates/core/src/lib.rs` (alphabetical order), then:
```bash
cargo test -p kuadrat-core store:: 2>&1 | grep -E 'error|cannot find'
```
Expected: FAIL — `cannot find type Store`.

- [ ] **Step 3: Write the store open + schema**

Prepend to `crates/core/src/store/mod.rs`:

```rust
//! SQLite-backed durable state for kuadrat: specs, deploy history, the durable
//! stage, per-app locks, and the event log.
//!
//! The store opens its own SQLite file directly, NOT through the `FileSystem`
//! seam. This is deliberate and sanctioned (ADR-0002): the database is
//! kuadrat's own state, not a side effect on a managed host. A future remote
//! executor keeps its store wherever kuadrat runs.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS apps (
    name       TEXT PRIMARY KEY,
    slug       TEXT NOT NULL UNIQUE,
    spec_json  TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS deploys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    app         TEXT NOT NULL,
    stage       TEXT NOT NULL,
    status      TEXT NOT NULL,
    detail      TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);
CREATE TABLE IF NOT EXISTS locks (
    app         TEXT PRIMARY KEY,
    deploy_id   INTEGER NOT NULL,
    acquired_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS events (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    deploy_id  INTEGER NOT NULL,
    stage      TEXT NOT NULL,
    status     TEXT NOT NULL,
    detail     TEXT,
    at         TEXT NOT NULL DEFAULT (datetime('now'))
);
";

/// Durable state. Synchronous; the async engine holds it as `&Store` and each
/// method locks the connection for its duration.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the store at `path`, running the schema. The
    /// schema uses `IF NOT EXISTS`, so opening an existing store is a no-op.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening store at {}", path.display()))?;
        conn.execute_batch(SCHEMA).context("initialising schema")?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core store::
```
Expected: 2 tests PASS.

- [ ] **Step 5: Amend ADR-0002 with the store carve-out**

In `docs/adr/0002-transport-agnostic-core.md`, in the **"What this costs"** section, extend the two-clause reviewer rule to three clauses. Replace the existing clause list with:

```markdown
  1. `tokio::process::Command` appears only in `exec::local`.
  2. `tokio::fs` appears only in `fs::local` — and neither does `std::fs`, nor
     `Path::exists()`, which is the same violation wearing a different name.
  3. **Exception — the store.** `store` opens its own SQLite file with
     `rusqlite::Connection::open` and creates the containing directory with
     `std::fs::create_dir_all`. This is not a host side effect: the database is
     kuadrat's own state, which stays wherever kuadrat runs, not on the managed
     host a remote executor would reach. It is the one sanctioned direct
     filesystem touch outside `fs::local`.
```

- [ ] **Step 6: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/store/mod.rs crates/core/src/lib.rs docs/adr/0002-transport-agnostic-core.md
git commit -m "feat(core): add Store with schema; record the store carve-out in ADR-0002"
```

---

### Task 4: Spec CRUD with slug-collision rejection

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store` (Task 3)
- Produces:
  - `Store::put_spec(&self, name: &str, slug: &str, spec_json: &str) -> Result<()>` — upsert keyed on `name`; errors if `slug` collides with a *different* app
  - `Store::current_spec(&self, name: &str) -> Result<Option<String>>` — the stored `spec_json`, or `None`

The store treats `spec_json` as opaque — it never parses it. `slug` is passed in by the caller (from `spec::slug`), keeping the store decoupled from spec internals.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/core/src/store/mod.rs`:

```rust
fn open_temp() -> (tempfile::TempDir, Store) {
    let dir = tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("k.db")).expect("open");
    (dir, store)
}

#[test]
fn spec_round_trips() {
    let (_dir, store) = open_temp();
    store.put_spec("web", "web", r#"{"name":"web"}"#).expect("put");
    assert_eq!(
        store.current_spec("web").expect("get").as_deref(),
        Some(r#"{"name":"web"}"#)
    );
}

#[test]
fn current_spec_is_none_for_an_unknown_app() {
    let (_dir, store) = open_temp();
    assert_eq!(store.current_spec("ghost").expect("get"), None);
}

#[test]
fn re_putting_the_same_app_updates_it() {
    let (_dir, store) = open_temp();
    store.put_spec("web", "web", r#"{"v":1}"#).expect("put1");
    store.put_spec("web", "web", r#"{"v":2}"#).expect("put2");
    assert_eq!(store.current_spec("web").expect("get").as_deref(), Some(r#"{"v":2}"#));
}

#[test]
fn a_colliding_slug_from_a_different_app_is_rejected() {
    let (_dir, store) = open_temp();
    store.put_spec("My App", "my-app", "{}").expect("first");
    let err = store.put_spec("my_app", "my-app", "{}").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("my-app"), "message was: {msg}");
    assert!(msg.contains("collides"), "message was: {msg}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core store:: 2>&1 | grep -E 'error|no method'
```
Expected: FAIL — `no method named put_spec`.

- [ ] **Step 3: Write the implementation**

Add these imports at the top of `crates/core/src/store/mod.rs` (merge with the existing `use` lines):

```rust
use anyhow::anyhow;
use rusqlite::{params, OptionalExtension};
```

Add to `impl Store`:

```rust
    /// Upsert the current spec for `name`. `slug` must be unique across apps;
    /// a collision with a different app is rejected with a clear error.
    /// `spec_json` is stored verbatim and never parsed.
    pub fn put_spec(&self, name: &str, slug: &str, spec_json: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO apps (name, slug, spec_json, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(name) DO UPDATE SET
               slug = excluded.slug,
               spec_json = excluded.spec_json,
               updated_at = datetime('now')",
            params![name, slug, spec_json],
        )
        .map_err(|e| {
            if e.to_string().contains("apps.slug") {
                anyhow!("slug {slug:?} for app {name:?} collides with an existing app")
            } else {
                anyhow::Error::from(e).context("storing spec")
            }
        })?;
        Ok(())
    }

    /// The current spec JSON for `name`, or `None`.
    pub fn current_spec(&self, name: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store lock");
        conn.query_row(
            "SELECT spec_json FROM apps WHERE name = ?1",
            params![name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("reading current spec")
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core store::
```
Expected: all store tests PASS (2 from Task 3 + 4 new).

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): store spec CRUD with slug-collision rejection"
```

---

### Task 5: Deploy lifecycle

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store` (Task 3), `deploy::{Stage, DeployStatus}` (Task 2)
- Produces:
  - `store::DeployRow { id: i64, app: String, stage: Stage, status: DeployStatus, detail: Option<String> }`
  - `Store::create_deploy(&self, app: &str) -> Result<i64>` — inserts a row at `stage=Detect, status=InProgress`, returns its id
  - `Store::advance_stage(&self, deploy_id: i64, stage: Stage) -> Result<()>` — errors if the deploy is not in progress
  - `Store::finish_deploy(&self, deploy_id: i64, status: DeployStatus, detail: Option<&str>) -> Result<()>`
  - `Store::deploy(&self, deploy_id: i64) -> Result<Option<DeployRow>>`
  - `Store::in_progress_deploys(&self) -> Result<Vec<DeployRow>>` — for reconciliation in G5

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/core/src/store/mod.rs`. Do **not** add a `use crate::deploy::...` line here — the test module's existing `use super::*` already re-exports `Stage` and `DeployStatus` once Task 5's implementation imports them at file level, and a second import trips `unused_imports` under `-D warnings`:

```rust
#[test]
fn a_new_deploy_starts_in_progress_at_detect() {
    let (_dir, store) = open_temp();
    let id = store.create_deploy("web").expect("create");
    let row = store.deploy(id).expect("get").expect("exists");
    assert_eq!(row.app, "web");
    assert_eq!(row.stage, Stage::Detect);
    assert_eq!(row.status, DeployStatus::InProgress);
}

#[test]
fn advancing_moves_the_stage_forward() {
    let (_dir, store) = open_temp();
    let id = store.create_deploy("web").expect("create");
    store.advance_stage(id, Stage::Build).expect("advance");
    assert_eq!(store.deploy(id).expect("get").expect("row").stage, Stage::Build);
}

#[test]
fn advancing_a_finished_deploy_errors() {
    let (_dir, store) = open_temp();
    let id = store.create_deploy("web").expect("create");
    store.finish_deploy(id, DeployStatus::Done, None).expect("finish");
    let err = store.advance_stage(id, Stage::Build).unwrap_err();
    assert!(err.to_string().contains(&id.to_string()));
}

#[test]
fn finishing_records_status_and_detail() {
    let (_dir, store) = open_temp();
    let id = store.create_deploy("web").expect("create");
    store
        .finish_deploy(id, DeployStatus::RolledBack, Some("healthcheck timed out"))
        .expect("finish");
    let row = store.deploy(id).expect("get").expect("row");
    assert_eq!(row.status, DeployStatus::RolledBack);
    assert_eq!(row.detail.as_deref(), Some("healthcheck timed out"));
}

#[test]
fn in_progress_lists_only_unfinished_deploys() {
    let (_dir, store) = open_temp();
    let a = store.create_deploy("a").expect("a");
    let _b = store.create_deploy("b").expect("b");
    store.finish_deploy(a, DeployStatus::Done, None).expect("finish a");

    let live = store.in_progress_deploys().expect("list");
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].app, "b");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core store:: 2>&1 | grep -E 'error|no method|cannot find'
```
Expected: FAIL — `cannot find type DeployRow` / `no method named create_deploy`.

- [ ] **Step 3: Write the implementation**

Add `use anyhow::bail;` to the imports (merge with the existing `anyhow` line, i.e. `use anyhow::{anyhow, bail, Context, Result};`).

Add the row type near the top of the file, after the `SCHEMA` constant:

```rust
use crate::deploy::{DeployStatus, Stage};

/// One row of deploy history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployRow {
    pub id: i64,
    pub app: String,
    pub stage: Stage,
    pub status: DeployStatus,
    pub detail: Option<String>,
}
```

Add to `impl Store`:

```rust
    /// Create a new deploy for `app`, at `Detect` / `InProgress`. Returns its id.
    pub fn create_deploy(&self, app: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO deploys (app, stage, status) VALUES (?1, ?2, ?3)",
            params![app, Stage::Detect.as_str(), DeployStatus::InProgress.as_str()],
        )
        .context("creating deploy")?;
        Ok(conn.last_insert_rowid())
    }

    /// Move an in-progress deploy to `stage`. Errors if it is already finished.
    pub fn advance_stage(&self, deploy_id: i64, stage: Stage) -> Result<()> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "UPDATE deploys SET stage = ?1 WHERE id = ?2 AND status = ?3",
                params![stage.as_str(), deploy_id, DeployStatus::InProgress.as_str()],
            )
            .context("advancing stage")?;
        if n == 0 {
            bail!("no in-progress deploy with id {deploy_id}");
        }
        Ok(())
    }

    /// Mark a deploy finished with a terminal status and optional detail.
    pub fn finish_deploy(
        &self,
        deploy_id: i64,
        status: DeployStatus,
        detail: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store lock");
        let n = conn
            .execute(
                "UPDATE deploys SET status = ?1, detail = ?2, finished_at = datetime('now')
                 WHERE id = ?3",
                params![status.as_str(), detail, deploy_id],
            )
            .context("finishing deploy")?;
        if n == 0 {
            bail!("no deploy with id {deploy_id}");
        }
        Ok(())
    }

    /// Read one deploy by id.
    pub fn deploy(&self, deploy_id: i64) -> Result<Option<DeployRow>> {
        let conn = self.conn.lock().expect("store lock");
        conn.query_row(
            "SELECT id, app, stage, status, detail FROM deploys WHERE id = ?1",
            params![deploy_id],
            deploy_row,
        )
        .optional()
        .context("reading deploy")?
        .transpose()
    }

    /// Every deploy still in progress — the reconciliation work-list (G5).
    pub fn in_progress_deploys(&self) -> Result<Vec<DeployRow>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id, app, stage, status, detail FROM deploys
                 WHERE status = ?1 ORDER BY id",
            )
            .context("preparing in-progress query")?;
        let rows = stmt
            .query_map(params![DeployStatus::InProgress.as_str()], deploy_row)
            .context("querying in-progress deploys")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading in-progress row")??);
        }
        Ok(out)
    }
```

Add these two free functions below `impl Store` (the row reader maps SQL columns; the builder parses the enum strings, so a corrupt stage/status surfaces as an error rather than a panic — split into two functions to avoid an immediately-invoked closure, which trips `clippy::redundant_closure_call` under `-D warnings`):

```rust
/// Read a `(id, app, stage, status, detail)` row. The outer `rusqlite::Result`
/// is the column read; the inner `anyhow::Result` is the enum parse.
fn deploy_row(row: &rusqlite::Row) -> rusqlite::Result<Result<DeployRow>> {
    Ok(build_deploy_row(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn build_deploy_row(
    id: i64,
    app: String,
    stage_s: String,
    status_s: String,
    detail: Option<String>,
) -> Result<DeployRow> {
    let stage = Stage::from_str(&stage_s)
        .ok_or_else(|| anyhow!("deploy {id} has unknown stage {stage_s:?}"))?;
    let status = DeployStatus::from_str(&status_s)
        .ok_or_else(|| anyhow!("deploy {id} has unknown status {status_s:?}"))?;
    Ok(DeployRow { id, app, stage, status, detail })
}
```

Note the `.transpose()` in `deploy()` and the `??` in `in_progress_deploys()`: `query_row`/`query_map` yield `rusqlite::Result<Result<DeployRow>>`, and these collapse the two error layers into one.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core store::
```
Expected: all store tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): store deploy lifecycle — create, advance, finish, query"
```

---

### Task 6: Per-app lock

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store` (Task 3)
- Produces:
  - `Store::acquire_lock(&self, app: &str, deploy_id: i64) -> Result<bool>` — `true` if acquired, `false` if already held
  - `Store::release_lock(&self, app: &str) -> Result<()>` — idempotent; releasing an unheld lock is not an error

The lock row is durable, so it survives a crash. G5's reconciliation is responsible for releasing a lock left held by a killed deploy.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/core/src/store/mod.rs`:

```rust
#[test]
fn a_lock_is_held_until_released() {
    let (_dir, store) = open_temp();
    assert!(store.acquire_lock("web", 1).expect("first"), "first acquire succeeds");
    assert!(!store.acquire_lock("web", 2).expect("second"), "second acquire is refused");

    store.release_lock("web").expect("release");
    assert!(store.acquire_lock("web", 3).expect("third"), "acquire after release succeeds");
}

#[test]
fn locks_are_per_app() {
    let (_dir, store) = open_temp();
    assert!(store.acquire_lock("a", 1).expect("a"));
    assert!(store.acquire_lock("b", 2).expect("b"), "a different app is not blocked");
}

#[test]
fn releasing_an_unheld_lock_is_ok() {
    let (_dir, store) = open_temp();
    store.release_lock("never-held").expect("no error");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core store:: 2>&1 | grep -E 'error|no method'
```
Expected: FAIL — `no method named acquire_lock`.

- [ ] **Step 3: Write the implementation**

Add to `impl Store`:

```rust
    /// Try to take the per-app lock. `true` if acquired, `false` if already
    /// held. The lock row is durable — a crash leaves it held, and G5's
    /// reconciliation releases it.
    pub fn acquire_lock(&self, app: &str, deploy_id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock");
        match conn.execute(
            "INSERT INTO locks (app, deploy_id) VALUES (?1, ?2)",
            params![app, deploy_id],
        ) {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::SqliteFailure(f, _))
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok(false)
            }
            Err(e) => Err(anyhow::Error::from(e).context("acquiring lock")),
        }
    }

    /// Release the per-app lock. Idempotent — releasing an unheld lock is fine.
    pub fn release_lock(&self, app: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute("DELETE FROM locks WHERE app = ?1", params![app])
            .context("releasing lock")?;
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core store::
```
Expected: all store tests PASS.

- [ ] **Step 5: Verify zero warnings**

```bash
cargo fmt && make check
```
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): per-app deploy lock in the store"
```

---

### Task 7: Event append and query

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store` (Task 3), `events::{Event, EventStatus}` and `deploy::Stage` (Task 2)
- Produces:
  - `Store::append_event(&self, event: &Event) -> Result<()>`
  - `Store::events_for(&self, deploy_id: i64) -> Result<Vec<Event>>` — in insertion order

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/core/src/store/mod.rs`. As in Task 5, add no `use crate::events::...` line — `use super::*` covers `Event` and `EventStatus` once the impl imports them at file level:

```rust
#[test]
fn events_append_and_read_back_in_order() {
    let (_dir, store) = open_temp();
    let id = store.create_deploy("web").expect("create");

    store
        .append_event(&Event {
            deploy_id: id,
            stage: Stage::Detect,
            status: EventStatus::Started,
            detail: None,
        })
        .expect("first");
    store
        .append_event(&Event {
            deploy_id: id,
            stage: Stage::Build,
            status: EventStatus::Failed,
            detail: Some("build broke".into()),
        })
        .expect("second");

    let events = store.events_for(id).expect("read");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].stage, Stage::Detect);
    assert_eq!(events[0].status, EventStatus::Started);
    assert_eq!(events[1].stage, Stage::Build);
    assert_eq!(events[1].detail.as_deref(), Some("build broke"));
}

#[test]
fn events_are_scoped_to_their_deploy() {
    let (_dir, store) = open_temp();
    let a = store.create_deploy("a").expect("a");
    let b = store.create_deploy("b").expect("b");
    store
        .append_event(&Event { deploy_id: a, stage: Stage::Detect, status: EventStatus::Started, detail: None })
        .expect("append a");

    assert_eq!(store.events_for(a).expect("a").len(), 1);
    assert_eq!(store.events_for(b).expect("b").len(), 0);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p kuadrat-core store:: 2>&1 | grep -E 'error|no method'
```
Expected: FAIL — `no method named append_event`.

- [ ] **Step 3: Write the implementation**

Add `use crate::events::{Event, EventStatus};` to the imports.

Add to `impl Store`:

```rust
    /// Append one event to a deploy's timeline.
    pub fn append_event(&self, event: &Event) -> Result<()> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO events (deploy_id, stage, status, detail) VALUES (?1, ?2, ?3, ?4)",
            params![
                event.deploy_id,
                event.stage.as_str(),
                event.status.as_str(),
                event.detail
            ],
        )
        .context("appending event")?;
        Ok(())
    }

    /// All events for a deploy, in insertion order.
    pub fn events_for(&self, deploy_id: i64) -> Result<Vec<Event>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT deploy_id, stage, status, detail FROM events
                 WHERE deploy_id = ?1 ORDER BY id",
            )
            .context("preparing events query")?;
        let rows = stmt
            .query_map(params![deploy_id], event_row)
            .context("querying events")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading event row")??);
        }
        Ok(out)
    }
```

Add these two free functions next to `deploy_row` (same split, same reason — no immediately-invoked closure):

```rust
/// Read an `(deploy_id, stage, status, detail)` event row.
fn event_row(row: &rusqlite::Row) -> rusqlite::Result<Result<Event>> {
    Ok(build_event(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn build_event(
    deploy_id: i64,
    stage_s: String,
    status_s: String,
    detail: Option<String>,
) -> Result<Event> {
    let stage = Stage::from_str(&stage_s)
        .ok_or_else(|| anyhow!("event for deploy {deploy_id} has unknown stage {stage_s:?}"))?;
    let status = EventStatus::from_str(&status_s)
        .ok_or_else(|| anyhow!("event for deploy {deploy_id} has unknown status {status_s:?}"))?;
    Ok(Event { deploy_id, stage, status, detail })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p kuadrat-core store::
```
Expected: all store tests PASS.

- [ ] **Step 5: Run the whole suite and the gate**

```bash
cargo fmt && make check && make test 2>&1 | grep -E '^test result'
```
Expected: `make check` clean; every test result line shows `0 failed`.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): append and query deploy events in the store"
```

---

## G1 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] `Store` is `Send + Sync` and holds no `host` parameter
- [ ] The store is the only new code touching the filesystem directly, and ADR-0002 records the carve-out
- [ ] A deploy can be created, advanced, finished, locked against, and have events appended — all surviving a reopen of the DB file
- [ ] `deploy/mod.rs` holds only the value types; no state-machine logic (that is G4)
- [ ] Spec JSON is stored opaque — the store never parses it

## Not in G1 (later task groups)

Detect + Build (G2), gateway + secrets + `run_with_stdin` (G3), the state-machine driver + compensation + restart-on-change (G4), reconciliation + acceptance (G5). The M1/M2 API-surface tidy-ups from `known-gaps.md` wait until the phase-2 public surface is complete, in G4 — folding them in now, while modules are still being added each group, would just be redone.
