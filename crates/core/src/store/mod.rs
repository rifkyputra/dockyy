//! SQLite-backed durable state for kuadrat: specs, deploy history, the durable
//! stage, per-app locks, and the event log.
//!
//! The store opens its own SQLite file directly, NOT through the `FileSystem`
//! seam. This is deliberate and sanctioned (ADR-0002): the database is
//! kuadrat's own state, not a side effect on a managed host. A future remote
//! executor keeps its store wherever kuadrat runs.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::deploy::{DeployStatus, Stage};
use crate::events::{Event, EventStatus};

// Inter-table references (deploys.app, events.deploy_id, locks.deploy_id) are
// unenforced by design in G1 — SQLite needs PRAGMA foreign_keys and explicit
// FK declarations, and nothing here deletes rows.
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

/// One row of deploy history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployRow {
    pub id: i64,
    pub app: String,
    pub stage: Stage,
    pub status: DeployStatus,
    pub detail: Option<String>,
}

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
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                anyhow!("slug {slug:?} for app {name:?} collides with an existing app")
            }
            other => anyhow::Error::from(other).context("storing spec"),
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

    /// Create a new deploy for `app`, at `Detect` / `InProgress`. Returns its id.
    pub fn create_deploy(&self, app: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock");
        conn.execute(
            "INSERT INTO deploys (app, stage, status) VALUES (?1, ?2, ?3)",
            params![
                app,
                Stage::Detect.as_str(),
                DeployStatus::InProgress.as_str()
            ],
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
}

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
    Ok(DeployRow {
        id,
        app,
        stage,
        status,
        detail,
    })
}

/// Read an `(deploy_id, stage, status, detail)` event row.
fn event_row(row: &rusqlite::Row) -> rusqlite::Result<Result<Event>> {
    Ok(build_event(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
    ))
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
    Ok(Event {
        deploy_id,
        stage,
        status,
        detail,
    })
}

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

    fn open_temp() -> (tempfile::TempDir, Store) {
        let dir = tempdir().expect("tempdir");
        let store = Store::open(&dir.path().join("k.db")).expect("open");
        (dir, store)
    }

    #[test]
    fn spec_round_trips() {
        let (_dir, store) = open_temp();
        store
            .put_spec("web", "web", r#"{"name":"web"}"#)
            .expect("put");
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
        assert_eq!(
            store.current_spec("web").expect("get").as_deref(),
            Some(r#"{"v":2}"#)
        );
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
        assert_eq!(
            store.deploy(id).expect("get").expect("row").stage,
            Stage::Build
        );
    }

    #[test]
    fn advancing_a_finished_deploy_errors() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");
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
        store
            .finish_deploy(a, DeployStatus::Done, None)
            .expect("finish a");

        let live = store.in_progress_deploys().expect("list");
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].app, "b");
    }

    #[test]
    fn a_lock_is_held_until_released() {
        let (_dir, store) = open_temp();
        assert!(
            store.acquire_lock("web", 1).expect("first"),
            "first acquire succeeds"
        );
        assert!(
            !store.acquire_lock("web", 2).expect("second"),
            "second acquire is refused"
        );

        store.release_lock("web").expect("release");
        assert!(
            store.acquire_lock("web", 3).expect("third"),
            "acquire after release succeeds"
        );
    }

    #[test]
    fn locks_are_per_app() {
        let (_dir, store) = open_temp();
        assert!(store.acquire_lock("a", 1).expect("a"));
        assert!(
            store.acquire_lock("b", 2).expect("b"),
            "a different app is not blocked"
        );
    }

    #[test]
    fn releasing_an_unheld_lock_is_ok() {
        let (_dir, store) = open_temp();
        store.release_lock("never-held").expect("no error");
    }

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
            .append_event(&Event {
                deploy_id: a,
                stage: Stage::Detect,
                status: EventStatus::Started,
                detail: None,
            })
            .expect("append a");

        assert_eq!(store.events_for(a).expect("a").len(), 1);
        assert_eq!(store.events_for(b).expect("b").len(), 0);
    }

    #[test]
    fn a_spec_a_deploy_and_a_held_lock_survive_a_reopen() {
        let dir = tempdir().expect("tempdir");
        let db = dir.path().join("persist.db");

        let store = Store::open(&db).expect("open");
        store
            .put_spec("web", "web", r#"{"v":1}"#)
            .expect("put spec");
        let id = store.create_deploy("web").expect("create deploy");
        assert!(
            store.acquire_lock("web", id).expect("acquire lock"),
            "lock should be free on first acquire"
        );
        drop(store);

        // Reopen the same path as a fresh Store, as a restarted process would.
        let reopened = Store::open(&db).expect("reopen");
        assert_eq!(
            reopened.current_spec("web").expect("get spec").as_deref(),
            Some(r#"{"v":1}"#),
            "spec should survive the reopen"
        );
        let row = reopened
            .deploy(id)
            .expect("get deploy")
            .expect("deploy row should exist after reopen");
        assert_eq!(row.app, "web");
        assert_eq!(row.id, id);
        assert!(
            !reopened.acquire_lock("web", 999).expect("acquire attempt"),
            "the lock acquired before the reopen should still be held"
        );
    }
}
