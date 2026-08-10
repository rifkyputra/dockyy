# kuadrat Phase 2 / G1 — Store and Events Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the SQLite-backed `store` module and the `events` type, so a deploy row can be created, advanced through stages, locked against, and have its events recorded.

**Architecture:** A synchronous `Store` wrapping a `rusqlite::Connection` behind a `Mutex`, opened at a path supplied by `Paths`. Four tables — `specs`, `deploys`, `locks`, `events`. The store is kuadrat's *own* state, so it opens SQLite directly rather than going through the `FileSystem` seam (see the design's store carve-out).

**Tech Stack:** Rust (edition 2021), rusqlite with the `bundled` feature, serde_json, anyhow, thiserror, tempfile for tests.

## Global Constraints

- **`kuadrat-core` never opens a socket and never takes a `host` parameter.** If any function grows one, the design has failed.
- **Every host command goes through the `Executor` trait; every host file operation through `FileSystem`.** `tokio::process::Command` appears only in `exec::local`; `tokio::fs` only in `fs::local`.
- **`store` is the one deliberate exception** — it opens a SQLite file directly. The database is kuadrat's own state, not a side effect on the managed host. Do not route it through `FileSystem`.
- **Paths are injectable.** No hardcoded `/etc/...` or `/var/lib/...` outside `Paths::default()`.
- **Secret values never appear** in specs, logs, error messages, or committed files. Specs carry secret *names* only.
- `make check && make test` must pass with **zero warnings** (`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`).
- Commit messages follow Conventional Commits and end with the trailer `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.
- **Out of scope for G1** (do not build): the deploy state machine driver, compensation, reconciliation, `detect`, `build`, `gateway`, `secrets`, `run_with_stdin`, HTTP, MCP. G1 is storage and types only.

---

### Task 1: API surface tidy, rusqlite dependency, and the store path

Closes two items carried in from `known-gaps.md` (`Paths` reachable by two public paths; no crate-root API surface) while the surface is still small, and adds the dependency and path G1 needs.

**Files:**
- Modify: `crates/core/Cargo.toml`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/src/workloads/apply.rs` (remove the `Paths` re-export)
- Modify: `crates/core/src/workloads/paths.rs` (add `db_path`)
- Modify: `crates/core/src/workloads/query.rs`, `crates/cli/src/main.rs` (import sites)

**Interfaces:**
- Consumes: existing `Paths { quadlet_dir }`
- Produces:
  - `Paths { quadlet_dir: PathBuf, db_path: PathBuf }`; `Paths::default()` → `db_path = /var/lib/kuadrat/kuadrat.db`; `Paths::rooted(root)` → `root.join("var/lib/kuadrat/kuadrat.db")`
  - Crate-root re-exports: `kuadrat_core::{WorkloadSpec, RestartPolicy, Paths, Executor, FileSystem}`
  - `workloads::apply::Paths` no longer exists — the canonical path is `workloads::paths::Paths`, re-exported at the crate root

- [ ] **Step 1: Add rusqlite**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd crates/core
cargo add rusqlite --features bundled
cargo add --dev tempfile   # already present; no-op if so
```

Use whatever version cargo resolves. `bundled` compiles SQLite into the binary, so no system `libsqlite3-dev` is needed. Record the resolved version in your report.

- [ ] **Step 2: Add `db_path` to `Paths`**

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
            db_path: root.join("var/lib/kuadrat/kuadrat.db"),
        }
    }
}
```

- [ ] **Step 3: Remove the duplicate `Paths` path and add crate-root re-exports**

In `crates/core/src/workloads/apply.rs`, delete this line:

```rust
pub use crate::workloads::paths::{unit_name, unit_path, Paths};
```

and replace it with a plain import so the module still compiles:

```rust
use crate::workloads::paths::{unit_name, unit_path, Paths};
```

In `crates/core/src/lib.rs`, add the canonical surface:

```rust
pub mod exec;
pub mod fs;
pub mod spec;
pub mod workloads;

pub use exec::Executor;
pub use fs::FileSystem;
pub use spec::{RestartPolicy, WorkloadSpec};
pub use workloads::paths::Paths;
```

- [ ] **Step 4: Fix every import site**

`cargo build` will name them. Expect `crates/core/src/workloads/query.rs` (its test module imports `Paths` from `apply`) and `crates/cli/src/main.rs`. Point them all at `kuadrat_core::Paths` (CLI) or `crate::workloads::paths::Paths` (inside the crate). Do not re-add a re-export to `apply`.

- [ ] **Step 5: Verify the whole suite still passes**

Run: `make check && make test`
Expected: 45 tests pass, zero warnings. No test logic should change — this task is purely a surface move plus a new field.

- [ ] **Step 6: Commit**

```bash
git add crates/core/Cargo.toml crates/core/Cargo.lock crates/core/src crates/cli/src
git commit -m "refactor(core): single canonical Paths, crate-root re-exports, db_path"
```

---

### Task 2: The `Stage` enum

`Stage` is a deploy concept, but the store must persist it, so it lands in G1. G4 adds the machine that drives it.

**Files:**
- Create: `crates/core/src/deploy/mod.rs`
- Create: `crates/core/src/deploy/stage.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: nothing
- Produces:
  - `pub enum Stage { Detect, Build, Secrets, Apply, Route, Healthcheck }`
  - `Stage::as_str(&self) -> &'static str` → `"detect"`, `"build"`, `"secrets"`, `"apply"`, `"route"`, `"healthcheck"`
  - `Stage::from_str(&str) -> Option<Stage>` (inherent method, **not** the `FromStr` trait — the store needs a plain `Option` return)
  - `Stage::ALL: [Stage; 6]` in execution order
  - `pub enum DeployStatus { InProgress, Done, RolledBack, Failed }` with the same `as_str` / `from_str` / `ALL` shape

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/deploy/stage.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_round_trips_through_its_string_form() {
        for stage in Stage::ALL {
            let s = stage.as_str();
            assert_eq!(Stage::from_str(s), Some(stage), "round trip failed for {s}");
        }
    }

    #[test]
    fn stage_all_is_in_execution_order() {
        assert_eq!(
            Stage::ALL,
            [
                Stage::Detect,
                Stage::Build,
                Stage::Secrets,
                Stage::Apply,
                Stage::Route,
                Stage::Healthcheck,
            ]
        );
    }

    #[test]
    fn unknown_stage_string_is_none() {
        assert_eq!(Stage::from_str("nonsense"), None);
        assert_eq!(Stage::from_str(""), None);
    }

    #[test]
    fn deploy_status_round_trips_through_its_string_form() {
        for status in DeployStatus::ALL {
            let s = status.as_str();
            assert_eq!(DeployStatus::from_str(s), Some(status), "round trip failed for {s}");
        }
    }

    #[test]
    fn unknown_status_string_is_none() {
        assert_eq!(DeployStatus::from_str("nonsense"), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Create `crates/core/src/deploy/mod.rs` containing `pub mod stage;`, add `pub mod deploy;` to `crates/core/src/lib.rs`, then run:
`cargo test -p kuadrat-core stage`
Expected: FAIL — `cannot find type Stage`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/deploy/stage.rs`:

```rust
use serde::{Deserialize, Serialize};

/// One step of the deploy loop. Persisted after every transition — the stored
/// value is what crash reconciliation resumes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Detect,
    Build,
    Secrets,
    Apply,
    Route,
    Healthcheck,
}

impl Stage {
    /// In execution order.
    pub const ALL: [Stage; 6] = [
        Stage::Detect,
        Stage::Build,
        Stage::Secrets,
        Stage::Apply,
        Stage::Route,
        Stage::Healthcheck,
    ];

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

    /// Inherent, not the `FromStr` trait: the store wants a plain `Option`.
    pub fn from_str(s: &str) -> Option<Stage> {
        Stage::ALL.into_iter().find(|stage| stage.as_str() == s)
    }
}

/// Terminal and in-flight states of a deploy.
///
/// `RolledBack` means a stage failed *and compensation succeeded* — the old
/// version is serving. `Failed` means compensation also failed, so host state
/// is unknown and nothing is retried automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeployStatus {
    InProgress,
    Done,
    RolledBack,
    Failed,
}

impl DeployStatus {
    pub const ALL: [DeployStatus; 4] = [
        DeployStatus::InProgress,
        DeployStatus::Done,
        DeployStatus::RolledBack,
        DeployStatus::Failed,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DeployStatus::InProgress => "in_progress",
            DeployStatus::Done => "done",
            DeployStatus::RolledBack => "rolled_back",
            DeployStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<DeployStatus> {
        DeployStatus::ALL.into_iter().find(|status| status.as_str() == s)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core stage`
Expected: 5 tests PASS.

Note: clippy may flag `from_str` with `should_implement_trait`. If it does, add `#[allow(clippy::should_implement_trait)]` above **each** `from_str` with the comment `// Inherent Option-returning form; the store does not want FromStr's Result.` Do not rename the method and do not implement `FromStr` — later tasks call `Stage::from_str`.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/deploy crates/core/src/lib.rs
git commit -m "feat(core): add Stage and DeployStatus enums"
```

---

### Task 3: `Store` — open, schema, and migration

**Files:**
- Create: `crates/core/src/store/mod.rs`
- Create: `crates/core/src/store/schema.rs`
- Modify: `crates/core/src/lib.rs`

**Interfaces:**
- Consumes: `Paths::db_path`
- Produces:
  - `pub struct Store { conn: Mutex<Connection> }`
  - `Store::open(db_path: &Path) -> anyhow::Result<Store>` — creates parent directories, applies the schema, enables foreign keys and WAL
  - `Store::open_in_memory() -> anyhow::Result<Store>` — for tests that do not need a file
  - `schema::APPLY: &str` — the `CREATE TABLE IF NOT EXISTS` batch

- [ ] **Step 1: Write the schema**

Create `crates/core/src/store/schema.rs`:

```rust
/// Applied on every open. Every statement is `IF NOT EXISTS`, so opening an
/// existing database is a no-op and opening a new one creates it.
pub const APPLY: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS specs (
    app        TEXT PRIMARY KEY,
    slug       TEXT NOT NULL UNIQUE,
    spec_json  TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS deploys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    app         TEXT NOT NULL,
    spec_json   TEXT NOT NULL,
    stage       TEXT NOT NULL,
    status      TEXT NOT NULL,
    image       TEXT,
    cause       TEXT,
    started_at  TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS deploys_app_status ON deploys (app, status);

CREATE TABLE IF NOT EXISTS locks (
    app         TEXT PRIMARY KEY,
    deploy_id   INTEGER NOT NULL,
    acquired_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS events (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    deploy_id INTEGER NOT NULL REFERENCES deploys(id),
    stage     TEXT NOT NULL,
    status    TEXT NOT NULL,
    detail    TEXT,
    at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS events_deploy ON events (deploy_id);
"#;
```

Timestamps come from SQLite's `datetime('now')` rather than a Rust clock, so `core` needs no clock dependency and no clock seam.

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/store/mod.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_creates_the_database_and_its_parent_directories() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("var/lib/kuadrat/kuadrat.db");
        assert!(!db.exists());

        let _store = Store::open(&db).expect("open");
        assert!(db.exists(), "database file was not created");
    }

    #[test]
    fn open_is_idempotent_on_an_existing_database() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("kuadrat.db");

        let first = Store::open(&db).expect("first open");
        drop(first);
        let _second = Store::open(&db).expect("second open must not fail");
    }

    #[test]
    fn schema_creates_all_four_tables() {
        let store = Store::open_in_memory().expect("open");
        let conn = store.conn.lock().expect("conn lock");

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .expect("prepare");
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .map(|r| r.expect("row"))
            .collect();

        for expected in ["deploys", "events", "locks", "specs"] {
            assert!(names.contains(&expected.to_string()), "missing table {expected}; got {names:?}");
        }
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Add `pub mod store;` to `crates/core/src/lib.rs`, then run:
`cargo test -p kuadrat-core store`
Expected: FAIL — `cannot find type Store`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/store/mod.rs`:

```rust
pub mod schema;

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// kuadrat's own state: specs, deploy history, the durable stage, and the
/// per-app lock.
///
/// Opens SQLite directly rather than going through the `FileSystem` seam. This
/// is deliberate: the database is kuadrat's state, not a side effect on the
/// managed host. A future remote executor reaches the host; the database stays
/// where kuadrat runs.
///
/// Methods are synchronous. Queries here are single-row lookups on a local
/// file — microseconds — so blocking briefly is cheaper than `spawn_blocking`.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if absent) the database at `db_path`, applying the schema.
    pub fn open(db_path: &Path) -> Result<Store> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        let conn = Connection::open(db_path)
            .with_context(|| format!("opening {}", db_path.display()))?;

        Self::init(conn)
    }

    /// An ephemeral database. For tests that do not need a file on disk.
    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory().context("opening in-memory database")?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.execute_batch(schema::APPLY)
            .context("applying schema")?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core store`
Expected: 3 tests PASS.

- [ ] **Step 6: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 7: Amend ADR-0002 with the store carve-out**

The ADR currently states the no-direct-side-effects rule as two clauses. `store` opening SQLite
directly looks like a violation of clause 2 unless the exception is written down. In
`docs/adr/0002-transport-agnostic-core.md`, inside the **"No direct side effects outside the two
local implementations"** bullet, add a third clause after clause 2:

```markdown
  3. `store` opens SQLite directly, and that is not a violation. The database is kuadrat's *own*
     state, not a side effect on the managed host: a remote transport reaches the target machine
     while the database stays wherever kuadrat runs. Routing it through `FileSystem` would make a
     fleet driver scatter its own bookkeeping across every managed host.
```

Do not edit the ADR's Context or Decision sections — this is a Consequences clarification, and the
record of the phase-1 correction above it stays as written.

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/store crates/core/src/lib.rs docs/adr/0002-transport-agnostic-core.md
git commit -m "feat(core): add SQLite store with schema"
```

---

### Task 4: Spec persistence and slug-collision rejection

Closes the `known-gaps.md` slug-collision item: `"My App"`, `"my_app"`, and `"my-app"` all slug to `my-app`, and until now two distinct specs would silently target the same unit.

**Files:**
- Create: `crates/core/src/store/specs.rs`
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store`, `WorkloadSpec`, `spec::slug`
- Produces, all on `impl Store`:
  - `put_spec(&self, spec: &WorkloadSpec) -> Result<()>` — insert or update by app name; errors if a *different* app already owns the slug
  - `get_spec(&self, app: &str) -> Result<Option<WorkloadSpec>>`
  - `list_specs(&self) -> Result<Vec<String>>` — app names, alphabetical

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/store/specs.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use crate::spec::WorkloadSpec;
    use crate::store::Store;

    #[test]
    fn put_then_get_round_trips_a_spec() {
        let store = Store::open_in_memory().expect("open");
        let mut spec = WorkloadSpec::new("pbrain", "alpine");
        spec.ports.push("3000:3000".into());

        store.put_spec(&spec).expect("put");
        let got = store.get_spec("pbrain").expect("get").expect("present");

        assert_eq!(got, spec);
    }

    #[test]
    fn get_spec_is_none_for_an_unknown_app() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.get_spec("nobody").expect("get").is_none());
    }

    #[test]
    fn put_spec_updates_an_existing_app() {
        let store = Store::open_in_memory().expect("open");
        store
            .put_spec(&WorkloadSpec::new("pbrain", "alpine"))
            .expect("first put");
        store
            .put_spec(&WorkloadSpec::new("pbrain", "node:22-alpine"))
            .expect("second put");

        let got = store.get_spec("pbrain").expect("get").expect("present");
        assert_eq!(got.image, "node:22-alpine");
        assert_eq!(store.list_specs().expect("list").len(), 1);
    }

    #[test]
    fn put_spec_rejects_a_colliding_slug_from_a_different_app() {
        let store = Store::open_in_memory().expect("open");
        store
            .put_spec(&WorkloadSpec::new("My App", "alpine"))
            .expect("first put");

        // "my_app" slugs to "my-app" too — same unit file, different app name.
        let err = store
            .put_spec(&WorkloadSpec::new("my_app", "alpine"))
            .unwrap_err();
        let msg = err.to_string();

        assert!(msg.contains("my-app"), "message was: {msg}");
        assert!(msg.contains("My App"), "message was: {msg}");
    }

    #[test]
    fn list_specs_returns_app_names_alphabetically() {
        let store = Store::open_in_memory().expect("open");
        store.put_spec(&WorkloadSpec::new("zeta", "alpine")).expect("put");
        store.put_spec(&WorkloadSpec::new("alpha", "alpine")).expect("put");

        assert_eq!(
            store.list_specs().expect("list"),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `mod specs;` to `crates/core/src/store/mod.rs`, then run:
`cargo test -p kuadrat-core store::specs`
Expected: FAIL — `no method named put_spec`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/store/specs.rs`:

```rust
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::spec::{slug, WorkloadSpec};
use crate::store::Store;

impl Store {
    /// Insert or update the spec for `spec.name`.
    ///
    /// Rejects a spec whose slug is already owned by a *different* app: two app
    /// names can slug identically (`"My App"` and `"my_app"` both give
    /// `my-app`), and they would otherwise silently share one unit file.
    pub fn put_spec(&self, spec: &WorkloadSpec) -> Result<()> {
        let conn = self.conn.lock().expect("conn lock");
        let slug = slug(&spec.name);

        let owner: Option<String> = conn
            .query_row(
                "SELECT app FROM specs WHERE slug = ?1",
                params![slug],
                |row| row.get(0),
            )
            .optional()
            .context("checking slug ownership")?;

        if let Some(owner) = owner {
            if owner != spec.name {
                bail!(
                    "slug `{slug}` is already used by app `{owner}`; \
                     `{}` would target the same unit file",
                    spec.name
                );
            }
        }

        let json = serde_json::to_string(spec).context("serializing spec")?;
        conn.execute(
            "INSERT INTO specs (app, slug, spec_json, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(app) DO UPDATE SET
               slug = excluded.slug,
               spec_json = excluded.spec_json,
               updated_at = excluded.updated_at",
            params![spec.name, slug, json],
        )
        .context("writing spec")?;

        Ok(())
    }

    /// The current spec for `app`, if one is stored.
    pub fn get_spec(&self, app: &str) -> Result<Option<WorkloadSpec>> {
        let conn = self.conn.lock().expect("conn lock");
        let json: Option<String> = conn
            .query_row(
                "SELECT spec_json FROM specs WHERE app = ?1",
                params![app],
                |row| row.get(0),
            )
            .optional()
            .context("reading spec")?;

        match json {
            Some(json) => Ok(Some(
                serde_json::from_str(&json).context("deserializing spec")?,
            )),
            None => Ok(None),
        }
    }

    /// Every stored app name, alphabetical.
    pub fn list_specs(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("conn lock");
        let mut stmt = conn
            .prepare("SELECT app FROM specs ORDER BY app")
            .context("preparing spec list")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("listing specs")?
            .collect::<rusqlite::Result<Vec<String>>>()
            .context("reading spec list")?;
        Ok(names)
    }
}
```

Note: `self.conn` is private to the `store` module. Because `specs.rs` is a child module of `store`, it can reach it — this is why these `impl Store` blocks live inside `store/`, not elsewhere.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core store::specs`
Expected: 5 tests PASS.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store
git commit -m "feat(core): persist specs and reject colliding slugs"
```

---

### Task 5: Deploy rows — start, advance, finish, and find in-flight

**Files:**
- Create: `crates/core/src/store/deploys.rs`
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store`, `WorkloadSpec`, `Stage`, `DeployStatus`
- Produces:
  - `pub struct DeployRow { pub id: i64, pub app: String, pub spec: WorkloadSpec, pub stage: Stage, pub status: DeployStatus, pub image: Option<String>, pub cause: Option<String> }`
  - `start_deploy(&self, app: &str, spec: &WorkloadSpec) -> Result<i64>` — inserts at `Stage::Detect` / `DeployStatus::InProgress`, returns the id
  - `set_stage(&self, deploy_id: i64, stage: Stage) -> Result<()>`
  - `finish_deploy(&self, deploy_id: i64, status: DeployStatus, image: Option<&str>, cause: Option<&str>) -> Result<()>` — also stamps `finished_at`
  - `get_deploy(&self, deploy_id: i64) -> Result<Option<DeployRow>>`
  - `in_progress_deploys(&self) -> Result<Vec<DeployRow>>` — what reconciliation resumes from
  - `last_done_spec(&self, app: &str) -> Result<Option<WorkloadSpec>>` — the rollback target

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/store/deploys.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use crate::deploy::stage::{DeployStatus, Stage};
    use crate::spec::WorkloadSpec;
    use crate::store::Store;

    fn store_with_spec() -> (Store, WorkloadSpec) {
        let store = Store::open_in_memory().expect("open");
        let spec = WorkloadSpec::new("pbrain", "alpine");
        store.put_spec(&spec).expect("put");
        (store, spec)
    }

    #[test]
    fn start_deploy_begins_at_detect_and_in_progress() {
        let (store, spec) = store_with_spec();
        let id = store.start_deploy("pbrain", &spec).expect("start");

        let row = store.get_deploy(id).expect("get").expect("present");
        assert_eq!(row.id, id);
        assert_eq!(row.app, "pbrain");
        assert_eq!(row.stage, Stage::Detect);
        assert_eq!(row.status, DeployStatus::InProgress);
        assert_eq!(row.spec, spec);
        assert!(row.image.is_none());
    }

    #[test]
    fn set_stage_advances_the_durable_stage() {
        let (store, spec) = store_with_spec();
        let id = store.start_deploy("pbrain", &spec).expect("start");

        store.set_stage(id, Stage::Apply).expect("set stage");

        let row = store.get_deploy(id).expect("get").expect("present");
        assert_eq!(row.stage, Stage::Apply);
        assert_eq!(row.status, DeployStatus::InProgress);
    }

    #[test]
    fn finish_deploy_records_status_image_and_cause() {
        let (store, spec) = store_with_spec();
        let id = store.start_deploy("pbrain", &spec).expect("start");

        store
            .finish_deploy(id, DeployStatus::Done, Some("alpine:abc123"), None)
            .expect("finish");

        let row = store.get_deploy(id).expect("get").expect("present");
        assert_eq!(row.status, DeployStatus::Done);
        assert_eq!(row.image.as_deref(), Some("alpine:abc123"));
        assert!(row.cause.is_none());
    }

    #[test]
    fn in_progress_deploys_excludes_finished_ones() {
        let (store, spec) = store_with_spec();
        let done = store.start_deploy("pbrain", &spec).expect("start");
        store
            .finish_deploy(done, DeployStatus::Done, Some("img"), None)
            .expect("finish");
        let live = store.start_deploy("pbrain", &spec).expect("start");

        let rows = store.in_progress_deploys().expect("in progress");
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();

        assert_eq!(ids, vec![live]);
    }

    #[test]
    fn last_done_spec_returns_the_most_recent_successful_deploy() {
        let store = Store::open_in_memory().expect("open");

        let old = WorkloadSpec::new("pbrain", "alpine:1");
        store.put_spec(&old).expect("put");
        let a = store.start_deploy("pbrain", &old).expect("start");
        store
            .finish_deploy(a, DeployStatus::Done, Some("img1"), None)
            .expect("finish");

        let new = WorkloadSpec::new("pbrain", "alpine:2");
        let b = store.start_deploy("pbrain", &new).expect("start");
        store
            .finish_deploy(b, DeployStatus::Done, Some("img2"), None)
            .expect("finish");

        let got = store.last_done_spec("pbrain").expect("last").expect("present");
        assert_eq!(got.image, "alpine:2");
    }

    #[test]
    fn last_done_spec_ignores_rolled_back_and_failed_deploys() {
        let store = Store::open_in_memory().expect("open");

        let good = WorkloadSpec::new("pbrain", "alpine:good");
        store.put_spec(&good).expect("put");
        let a = store.start_deploy("pbrain", &good).expect("start");
        store
            .finish_deploy(a, DeployStatus::Done, Some("img"), None)
            .expect("finish");

        let bad = WorkloadSpec::new("pbrain", "alpine:bad");
        let b = store.start_deploy("pbrain", &bad).expect("start");
        store
            .finish_deploy(b, DeployStatus::RolledBack, None, Some("healthcheck timeout"))
            .expect("finish");

        let got = store.last_done_spec("pbrain").expect("last").expect("present");
        assert_eq!(
            got.image, "alpine:good",
            "a rolled-back deploy must never become the rollback target"
        );
    }

    #[test]
    fn last_done_spec_is_none_before_any_successful_deploy() {
        let (store, _spec) = store_with_spec();
        assert!(store.last_done_spec("pbrain").expect("last").is_none());
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `mod deploys;` to `crates/core/src/store/mod.rs`, then run:
`cargo test -p kuadrat-core store::deploys`
Expected: FAIL — `no method named start_deploy`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/store/deploys.rs`:

```rust
use anyhow::{anyhow, Context, Result};
use rusqlite::{params, OptionalExtension, Row};

use crate::deploy::stage::{DeployStatus, Stage};
use crate::spec::WorkloadSpec;
use crate::store::Store;

/// One row of the `deploys` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployRow {
    pub id: i64,
    pub app: String,
    pub spec: WorkloadSpec,
    pub stage: Stage,
    pub status: DeployStatus,
    pub image: Option<String>,
    pub cause: Option<String>,
}

const SELECT_COLUMNS: &str = "id, app, spec_json, stage, status, image, cause";

fn row_to_deploy(row: &Row<'_>) -> rusqlite::Result<DeployRow> {
    let spec_json: String = row.get(2)?;
    let stage: String = row.get(3)?;
    let status: String = row.get(4)?;

    // A row we cannot interpret is corruption, not a normal absence. Surface it
    // as an error rather than silently defaulting to a stage that would make
    // reconciliation unwind the wrong things.
    let spec = serde_json::from_str(&spec_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let stage = Stage::from_str(&stage).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("unknown stage `{stage}`"))),
        )
    })?;
    let status = DeployStatus::from_str(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("unknown status `{status}`"))),
        )
    })?;

    Ok(DeployRow {
        id: row.get(0)?,
        app: row.get(1)?,
        spec,
        stage,
        status,
        image: row.get(5)?,
        cause: row.get(6)?,
    })
}

impl Store {
    /// Begin a deploy at `Stage::Detect` / `DeployStatus::InProgress`.
    pub fn start_deploy(&self, app: &str, spec: &WorkloadSpec) -> Result<i64> {
        let conn = self.conn.lock().expect("conn lock");
        let json = serde_json::to_string(spec).context("serializing spec")?;

        conn.execute(
            "INSERT INTO deploys (app, spec_json, stage, status)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                app,
                json,
                Stage::Detect.as_str(),
                DeployStatus::InProgress.as_str()
            ],
        )
        .context("starting deploy")?;

        Ok(conn.last_insert_rowid())
    }

    /// Persist the stage a deploy has reached. Called after every transition.
    pub fn set_stage(&self, deploy_id: i64, stage: Stage) -> Result<()> {
        let conn = self.conn.lock().expect("conn lock");
        let changed = conn
            .execute(
                "UPDATE deploys SET stage = ?1 WHERE id = ?2",
                params![stage.as_str(), deploy_id],
            )
            .context("setting stage")?;

        if changed == 0 {
            return Err(anyhow!("no deploy with id {deploy_id}"));
        }
        Ok(())
    }

    /// Record a terminal status and stamp `finished_at`.
    pub fn finish_deploy(
        &self,
        deploy_id: i64,
        status: DeployStatus,
        image: Option<&str>,
        cause: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("conn lock");
        let changed = conn
            .execute(
                "UPDATE deploys
                 SET status = ?1, image = ?2, cause = ?3, finished_at = datetime('now')
                 WHERE id = ?4",
                params![status.as_str(), image, cause, deploy_id],
            )
            .context("finishing deploy")?;

        if changed == 0 {
            return Err(anyhow!("no deploy with id {deploy_id}"));
        }
        Ok(())
    }

    pub fn get_deploy(&self, deploy_id: i64) -> Result<Option<DeployRow>> {
        let conn = self.conn.lock().expect("conn lock");
        let sql = format!("SELECT {SELECT_COLUMNS} FROM deploys WHERE id = ?1");
        conn.query_row(&sql, params![deploy_id], row_to_deploy)
            .optional()
            .context("reading deploy")
    }

    /// Every deploy still `InProgress` — what reconciliation resumes from.
    pub fn in_progress_deploys(&self) -> Result<Vec<DeployRow>> {
        let conn = self.conn.lock().expect("conn lock");
        let sql = format!(
            "SELECT {SELECT_COLUMNS} FROM deploys WHERE status = ?1 ORDER BY id"
        );
        let mut stmt = conn.prepare(&sql).context("preparing in-progress query")?;
        let rows = stmt
            .query_map(params![DeployStatus::InProgress.as_str()], row_to_deploy)
            .context("querying in-progress deploys")?
            .collect::<rusqlite::Result<Vec<DeployRow>>>()
            .context("reading in-progress deploys")?;
        Ok(rows)
    }

    /// The spec of the most recent successful deploy — the rollback target.
    ///
    /// Only `Done` counts: a rolled-back or failed deploy must never become
    /// something a later rollback restores.
    pub fn last_done_spec(&self, app: &str) -> Result<Option<WorkloadSpec>> {
        let conn = self.conn.lock().expect("conn lock");
        let json: Option<String> = conn
            .query_row(
                "SELECT spec_json FROM deploys
                 WHERE app = ?1 AND status = ?2
                 ORDER BY id DESC LIMIT 1",
                params![app, DeployStatus::Done.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("reading last successful deploy")?;

        match json {
            Some(json) => Ok(Some(
                serde_json::from_str(&json).context("deserializing spec")?,
            )),
            None => Ok(None),
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core store::deploys`
Expected: 7 tests PASS.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store
git commit -m "feat(core): persist deploy rows, stages, and the rollback target"
```

---

### Task 6: The per-app lock

The design's hard requirement: *"If kuadrat dies mid-deploy the lock row stays held, and every future deploy of that app is rejected forever."* This task provides the primitives; G5's reconciliation is what calls `release_lock` for a crashed deploy.

**Files:**
- Create: `crates/core/src/store/locks.rs`
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store`
- Produces:
  - `try_acquire_lock(&self, app: &str, deploy_id: i64) -> Result<bool>` — `true` if acquired, `false` if already held. **Not an error** — a rejected concurrent deploy is a normal outcome.
  - `release_lock(&self, app: &str) -> Result<()>` — idempotent; releasing an unheld lock is fine
  - `lock_holder(&self, app: &str) -> Result<Option<i64>>` — the deploy id holding it

- [ ] **Step 1: Write the failing test**

Create `crates/core/src/store/locks.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn lock_is_acquired_when_free() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.try_acquire_lock("pbrain", 1).expect("acquire"));
        assert_eq!(store.lock_holder("pbrain").expect("holder"), Some(1));
    }

    #[test]
    fn second_acquire_is_refused_not_an_error() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.try_acquire_lock("pbrain", 1).expect("first"));

        // A concurrent deploy is a normal outcome, not a failure.
        assert!(!store.try_acquire_lock("pbrain", 2).expect("second must not error"));
        assert_eq!(
            store.lock_holder("pbrain").expect("holder"),
            Some(1),
            "the original holder must not be displaced"
        );
    }

    #[test]
    fn releasing_frees_the_lock_for_the_next_deploy() {
        let store = Store::open_in_memory().expect("open");
        store.try_acquire_lock("pbrain", 1).expect("first");
        store.release_lock("pbrain").expect("release");

        assert!(store.lock_holder("pbrain").expect("holder").is_none());
        assert!(store.try_acquire_lock("pbrain", 2).expect("second"));
    }

    #[test]
    fn releasing_an_unheld_lock_is_not_an_error() {
        let store = Store::open_in_memory().expect("open");
        store.release_lock("never-locked").expect("release must be idempotent");
    }

    #[test]
    fn locks_are_per_app() {
        let store = Store::open_in_memory().expect("open");
        assert!(store.try_acquire_lock("alpha", 1).expect("alpha"));
        assert!(
            store.try_acquire_lock("beta", 2).expect("beta"),
            "one app's lock must not block another's"
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Add `mod locks;` to `crates/core/src/store/mod.rs`, then run:
`cargo test -p kuadrat-core store::locks`
Expected: FAIL — `no method named try_acquire_lock`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/core/src/store/locks.rs`:

```rust
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};

use crate::store::Store;

impl Store {
    /// Take the deploy lock for `app`.
    ///
    /// Returns `false` when it is already held — a concurrent deploy is
    /// rejected, not queued, and rejection is a normal outcome rather than an
    /// error. `INSERT OR IGNORE` against the primary key makes the check and
    /// the take one atomic statement, so two callers cannot both succeed.
    pub fn try_acquire_lock(&self, app: &str, deploy_id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("conn lock");
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO locks (app, deploy_id) VALUES (?1, ?2)",
                params![app, deploy_id],
            )
            .context("acquiring lock")?;

        Ok(inserted == 1)
    }

    /// Release the lock. Idempotent: releasing an unheld lock succeeds.
    ///
    /// Must be called on every exit path — success, rollback, and
    /// reconciliation of a crashed deploy. A leaked lock rejects every future
    /// deploy of that app permanently.
    pub fn release_lock(&self, app: &str) -> Result<()> {
        let conn = self.conn.lock().expect("conn lock");
        conn.execute("DELETE FROM locks WHERE app = ?1", params![app])
            .context("releasing lock")?;
        Ok(())
    }

    /// The deploy id currently holding `app`'s lock, if any.
    pub fn lock_holder(&self, app: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().expect("conn lock");
        conn.query_row(
            "SELECT deploy_id FROM locks WHERE app = ?1",
            params![app],
            |row| row.get(0),
        )
        .optional()
        .context("reading lock holder")
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core store::locks`
Expected: 5 tests PASS.

- [ ] **Step 5: Verify zero warnings**

Run: `make check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store
git commit -m "feat(core): add the per-app deploy lock"
```

---

### Task 7: Events

**Files:**
- Create: `crates/core/src/events/mod.rs`
- Create: `crates/core/src/store/events.rs`
- Modify: `crates/core/src/lib.rs`, `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes: `Store`, `Stage`, `DeployStatus`
- Produces:
  - `pub enum EventStatus { Started, Succeeded, Failed }` with `as_str` / `from_str` / `ALL`
  - `pub struct Event { pub deploy_id: i64, pub stage: Stage, pub status: EventStatus, pub detail: Option<String> }`
  - `append_event(&self, event: &Event) -> Result<()>`
  - `events_for(&self, deploy_id: i64) -> Result<Vec<Event>>` — in insertion order

- [ ] **Step 1: Write the `Event` type**

Create `crates/core/src/events/mod.rs`:

```rust
use serde::{Deserialize, Serialize};

use crate::deploy::stage::Stage;

/// What happened at a stage transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventStatus {
    Started,
    Succeeded,
    Failed,
}

impl EventStatus {
    pub const ALL: [EventStatus; 3] = [
        EventStatus::Started,
        EventStatus::Succeeded,
        EventStatus::Failed,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            EventStatus::Started => "started",
            EventStatus::Succeeded => "succeeded",
            EventStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<EventStatus> {
        EventStatus::ALL.into_iter().find(|st| st.as_str() == s)
    }
}

/// One stage transition. The integration point for all three phase-3/4
/// surfaces: the web UI streams these, the agent reads the failing one to
/// diagnose, and external subscribers receive them over a webhook.
///
/// `detail` is operator-facing text. It must never contain a secret value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub deploy_id: i64,
    pub stage: Stage,
    pub status: EventStatus,
    pub detail: Option<String>,
}
```

- [ ] **Step 2: Write the failing test**

Create `crates/core/src/store/events.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use crate::deploy::stage::Stage;
    use crate::events::{Event, EventStatus};
    use crate::spec::WorkloadSpec;
    use crate::store::Store;

    fn store_with_deploy() -> (Store, i64) {
        let store = Store::open_in_memory().expect("open");
        let spec = WorkloadSpec::new("pbrain", "alpine");
        store.put_spec(&spec).expect("put");
        let id = store.start_deploy("pbrain", &spec).expect("start");
        (store, id)
    }

    #[test]
    fn events_round_trip_in_insertion_order() {
        let (store, id) = store_with_deploy();

        let first = Event {
            deploy_id: id,
            stage: Stage::Detect,
            status: EventStatus::Started,
            detail: None,
        };
        let second = Event {
            deploy_id: id,
            stage: Stage::Build,
            status: EventStatus::Failed,
            detail: Some("no Containerfile".into()),
        };

        store.append_event(&first).expect("append first");
        store.append_event(&second).expect("append second");

        assert_eq!(store.events_for(id).expect("read"), vec![first, second]);
    }

    #[test]
    fn events_are_scoped_to_their_deploy() {
        let (store, first_id) = store_with_deploy();
        let spec = store.get_spec("pbrain").expect("get").expect("present");
        let second_id = store.start_deploy("pbrain", &spec).expect("start");

        store
            .append_event(&Event {
                deploy_id: first_id,
                stage: Stage::Apply,
                status: EventStatus::Succeeded,
                detail: None,
            })
            .expect("append");

        assert_eq!(store.events_for(first_id).expect("read").len(), 1);
        assert!(store.events_for(second_id).expect("read").is_empty());
    }

    #[test]
    fn event_status_round_trips_through_its_string_form() {
        for status in EventStatus::ALL {
            assert_eq!(EventStatus::from_str(status.as_str()), Some(status));
        }
        assert_eq!(EventStatus::from_str("nonsense"), None);
    }

    #[test]
    fn appending_an_event_for_an_unknown_deploy_is_rejected() {
        let store = Store::open_in_memory().expect("open");
        // The events table has a foreign key to deploys(id).
        let err = store
            .append_event(&Event {
                deploy_id: 9999,
                stage: Stage::Detect,
                status: EventStatus::Started,
                detail: None,
            })
            .unwrap_err();
        assert!(err.to_string().to_lowercase().contains("event"), "message was: {err}");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Add `pub mod events;` to `crates/core/src/lib.rs` and `mod events;` to `crates/core/src/store/mod.rs`, then run:
`cargo test -p kuadrat-core store::events`
Expected: FAIL — `no method named append_event`.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/core/src/store/events.rs`:

```rust
use anyhow::{Context, Result};
use rusqlite::{params, Row};

use crate::deploy::stage::Stage;
use crate::events::{Event, EventStatus};
use crate::store::Store;

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<Event> {
    let stage: String = row.get(1)?;
    let status: String = row.get(2)?;

    let stage = Stage::from_str(&stage).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("unknown stage `{stage}`"))),
        )
    })?;
    let status = EventStatus::from_str(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!("unknown status `{status}`"))),
        )
    })?;

    Ok(Event {
        deploy_id: row.get(0)?,
        stage,
        status,
        detail: row.get(3)?,
    })
}

impl Store {
    /// Record a stage transition.
    pub fn append_event(&self, event: &Event) -> Result<()> {
        let conn = self.conn.lock().expect("conn lock");
        conn.execute(
            "INSERT INTO events (deploy_id, stage, status, detail)
             VALUES (?1, ?2, ?3, ?4)",
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

    /// Every event for a deploy, in insertion order.
    pub fn events_for(&self, deploy_id: i64) -> Result<Vec<Event>> {
        let conn = self.conn.lock().expect("conn lock");
        let mut stmt = conn
            .prepare(
                "SELECT deploy_id, stage, status, detail FROM events
                 WHERE deploy_id = ?1 ORDER BY id",
            )
            .context("preparing event query")?;
        let events = stmt
            .query_map(params![deploy_id], row_to_event)
            .context("querying events")?
            .collect::<rusqlite::Result<Vec<Event>>>()
            .context("reading events")?;
        Ok(events)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p kuadrat-core store::events`
Expected: 4 tests PASS.

If `appending_an_event_for_an_unknown_deploy_is_rejected` fails because the insert *succeeded*, foreign keys are not enforced. `PRAGMA foreign_keys = ON` must be executed on the connection, and rusqlite resets pragmas per connection — verify it is inside `schema::APPLY` and that `execute_batch` ran it. Do not weaken the test.

- [ ] **Step 6: Run the full suite and verify zero warnings**

Run: `make check && make test`
Expected: all tests pass (45 from phase 1 plus 29 added here), zero warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/events crates/core/src/store crates/core/src/lib.rs
git commit -m "feat(core): add typed events and their store"
```

---

## G1 completion checklist

- [ ] `make check && make test` passes with zero warnings
- [ ] A deploy row can be created, advanced through stages, and finished
- [ ] `try_acquire_lock` returns `false` rather than erroring on a held lock
- [ ] `release_lock` is idempotent
- [ ] `last_done_spec` ignores rolled-back and failed deploys
- [ ] A colliding slug from a different app is rejected with both names in the message
- [ ] No `kuadrat-core` function takes a `host` parameter
- [ ] `tokio::process::Command` only in `exec::local`; `tokio::fs` only in `fs::local`
- [ ] `store` is the only module opening SQLite directly, and ADR-0002 has been amended with the store carve-out as a third clause

## What G2 adds

`Detect` and `Build`: Containerfile discovery, `git rev-parse HEAD` through the `Executor` seam for the image tag, and `podman build`. Done when a repo path yields a tagged image.
