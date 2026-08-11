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
use serde::{Deserialize, Serialize};

use crate::deploy::{DeployStatus, Stage};
use crate::events::{Event, EventKind, EventStatus, StoredEvent, DEPLOY_ROW};
use crate::spec::Route;

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
CREATE TABLE IF NOT EXISTS app_config (
    name         TEXT PRIMARY KEY,
    repo_path    TEXT NOT NULL,
    route_domain TEXT,
    route_port   INTEGER,
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
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

/// What the operator asked for: where an app's source lives, and optionally
/// the domain it should be served on.
///
/// Distinct from the `apps` row, which records what was actually deployed. A
/// registration exists from the moment someone adds the app; a deployed spec
/// only exists after a deploy succeeds. Keeping them apart is what lets an app
/// be registered before it has ever been built.
///
/// **Authority rule:** `app_config` is the operator's intent and is
/// authoritative for `repo_path` and `route`; `apps` is the deploy record and
/// is authoritative for `image` and the resolved spec. When an `app_config`
/// row exists, a deploy must assign `spec.route = config.route`
/// unconditionally — including `None` — rather than routing it through
/// `resolve_spec`'s `route_override` parameter, where `None` means "no
/// override" (keep whatever the repo or stored spec already carries) rather
/// than "no route". Passing `config.route` as that override would make
/// clearing a route in the UI silently ineffective: the next deploy would
/// re-apply the old route from the stored spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub name: String,
    pub repo_path: String,
    pub route: Option<Route>,
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

    /// Atomically finish a deploy and append its terminal event, in one
    /// SQLite transaction.
    ///
    /// `finish_deploy` and `append_event` are two separate writes/locks. The
    /// SSE handler reads the `deploys` row and the event backlog as two
    /// separate queries; if a reader's row read lands after the status write
    /// but its backlog read lands before the event append, it sees a terminal
    /// row with no terminal event in the backlog. That is supposed to mean
    /// "this deploy ended by a path that appends no event" (`reserve`
    /// rejecting a duplicate is the only one) — a reader must never confuse
    /// that with "the event just hasn't landed yet". Wrapping both writes in
    /// one transaction removes the in-between state: a reader sees either
    /// `in_progress` with no terminal event, or a terminal row with its event
    /// — never the state where the row says done and the log hasn't caught up.
    ///
    /// `reserve`'s rejection path must keep calling plain `finish_deploy`, not
    /// this: it deliberately writes no event, because the caller gets `Err`
    /// and a 409 and is never handed the id to watch.
    pub fn finish_deploy_with_event(
        &self,
        deploy_id: i64,
        status: DeployStatus,
        detail: Option<&str>,
    ) -> Result<StoredEvent> {
        let mut conn = self.conn.lock().expect("store lock");
        let tx = conn
            .transaction()
            .context("starting finish-deploy-with-event transaction")?;

        let n = tx
            .execute(
                "UPDATE deploys SET status = ?1, detail = ?2, finished_at = datetime('now')
                 WHERE id = ?3",
                params![status.as_str(), detail, deploy_id],
            )
            .context("finishing deploy")?;
        if n == 0 {
            bail!("no deploy with id {deploy_id}");
        }

        let event = Event::finished(deploy_id, status, detail.map(str::to_string));
        let (stage, ev_status) = event.kind.columns();
        let (id, at) = tx
            .query_row(
                "INSERT INTO events (deploy_id, stage, status, detail)
                 VALUES (?1, ?2, ?3, ?4) RETURNING id, at",
                params![event.deploy_id, stage, ev_status, event.detail],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .context("appending terminal event")?;

        tx.commit()
            .context("committing finish-deploy-with-event transaction")?;

        Ok(StoredEvent { id, at, event })
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

    /// An app's deploy history, newest first, bounded by `limit`.
    ///
    /// Ordered by id rather than by `created_at`: ids are monotonic from
    /// SQLite's `AUTOINCREMENT`, while two deploys created inside the same
    /// second share a timestamp and would order arbitrarily.
    pub fn recent_deploys(&self, app: &str, limit: usize) -> Result<Vec<DeployRow>> {
        let conn = self.conn.lock().expect("store lock");
        // Saturating, not `as`: `limit as i64` wraps a `usize` above `i64::MAX`
        // to a negative number, and SQLite reads a negative LIMIT as *no* limit —
        // so the value that most obviously means "everything" would be the one
        // that silently removes the bound. Bounded reads are a rule here, not a
        // preference; `logs::tail` clamps its line count for the same reason.
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn
            .prepare(
                "SELECT id, app, stage, status, detail FROM deploys
                 WHERE app = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .context("preparing recent deploys query")?;
        let rows = stmt
            .query_map(params![app, limit], deploy_row)
            .context("querying recent deploys")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading deploy row")??);
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

    /// Append one event and return it as stored: the same event, plus the id
    /// and insert timestamp SQLite assigned. That id is what the SSE stream
    /// deduplicates and resumes on, so it must come from the same insert
    /// rather than being counted by the caller — `RETURNING` gets both the id
    /// and the timestamp in the one round trip that created them.
    pub fn append_event(&self, event: &Event) -> Result<StoredEvent> {
        let conn = self.conn.lock().expect("store lock");
        let (stage, status) = event.kind.columns();
        let (id, at) = conn
            .query_row(
                "INSERT INTO events (deploy_id, stage, status, detail)
                 VALUES (?1, ?2, ?3, ?4) RETURNING id, at",
                params![event.deploy_id, stage, status, event.detail],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .context("appending event")?;
        Ok(StoredEvent {
            id,
            at,
            event: event.clone(),
        })
    }

    /// All events for a deploy, in insertion order, each with its id.
    pub fn events_for(&self, deploy_id: i64) -> Result<Vec<StoredEvent>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id, at, deploy_id, stage, status, detail FROM events
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

    /// Register an app, or replace an existing registration.
    ///
    /// The upsert writes every column unconditionally, including the route
    /// columns when the route is `None`. Writing only the non-null values
    /// would make clearing a route impossible — the app would keep serving on
    /// a domain the operator had just removed.
    ///
    /// Rejects a `name` that would clobber another app's units: an empty
    /// slug, or a slug that collides with a *different* existing name in
    /// either `app_config` or `apps`. The slug is the filesystem/systemd
    /// identity — it derives the Quadlet unit filename, the container name,
    /// and the image tag — so a collision here means the second app's deploy
    /// overwrites the first app's unit before `put_spec`'s own collision
    /// guard ever runs. Also rejects a relative `repo_path`: the daemon's
    /// working directory is not the operator's shell, so a relative path
    /// would resolve against the wrong place.
    pub fn register_app(&self, config: &AppConfig) -> Result<()> {
        let slug = crate::spec::slug(&config.name);
        if slug.is_empty() {
            bail!(
                "app name {:?} yields an empty identifier; it needs at least one \
                 letter or digit",
                config.name
            );
        }
        if !Path::new(&config.repo_path).is_absolute() {
            bail!(
                "repo_path {:?} for app {:?} is not absolute; the daemon's working \
                 directory is not the operator's shell, so a relative path resolves \
                 against the wrong place",
                config.repo_path,
                config.name
            );
        }

        let (domain, port) = match &config.route {
            Some(route) => (Some(route.domain.as_str()), Some(route.port as i64)),
            None => (None, None),
        };
        let conn = self.conn.lock().expect("store lock");

        // Collision check across both tables, excluding this app's own
        // existing row — re-registering the same name must still succeed.
        let other_names: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT name FROM app_config WHERE name != ?1
                     UNION SELECT name FROM apps WHERE name != ?1",
                )
                .context("preparing collision check")?;
            let rows = stmt
                .query_map(params![config.name], |row| row.get::<_, String>(0))
                .context("querying existing names")?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("reading existing names")?
        };
        if other_names
            .iter()
            .any(|name| crate::spec::slug(name) == slug)
        {
            bail!(
                "slug {slug:?} for app {:?} collides with an existing app",
                config.name
            );
        }

        conn.execute(
            "INSERT INTO app_config (name, repo_path, route_domain, route_port, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(name) DO UPDATE SET
               repo_path    = excluded.repo_path,
               route_domain = excluded.route_domain,
               route_port   = excluded.route_port,
               updated_at   = datetime('now')",
            params![config.name, config.repo_path, domain, port],
        )
        .context("registering app")?;
        Ok(())
    }

    /// The registration for `name`, or `None` if it was never registered.
    pub fn app_config(&self, name: &str) -> Result<Option<AppConfig>> {
        let conn = self.conn.lock().expect("store lock");
        conn.query_row(
            "SELECT name, repo_path, route_domain, route_port FROM app_config WHERE name = ?1",
            params![name],
            app_config_row,
        )
        .optional()
        .context("reading app config")?
        .transpose()
    }

    /// Every registration, ordered by name. Ordered in SQL rather than by the
    /// caller so the app list is stable between requests.
    pub fn list_app_configs(&self) -> Result<Vec<AppConfig>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT name, repo_path, route_domain, route_port FROM app_config
                 ORDER BY name",
            )
            .context("preparing app config query")?;
        let rows = stmt
            .query_map([], app_config_row)
            .context("querying app configs")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading app config row")??);
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

fn event_row(row: &rusqlite::Row) -> rusqlite::Result<Result<StoredEvent>> {
    Ok(build_event(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn build_event(
    id: i64,
    at: String,
    deploy_id: i64,
    stage_s: String,
    status_s: String,
    detail: Option<String>,
) -> Result<StoredEvent> {
    let kind = if stage_s == DEPLOY_ROW {
        let status = DeployStatus::from_str(&status_s).ok_or_else(|| {
            anyhow!("event for deploy {deploy_id} has unknown status {status_s:?}")
        })?;
        if status == DeployStatus::InProgress {
            bail!("event for deploy {deploy_id} is deploy-level but not terminal");
        }
        EventKind::Finished { status }
    } else {
        let stage = Stage::from_str(&stage_s)
            .ok_or_else(|| anyhow!("event for deploy {deploy_id} has unknown stage {stage_s:?}"))?;
        let status = EventStatus::from_str(&status_s).ok_or_else(|| {
            anyhow!("event for deploy {deploy_id} has unknown status {status_s:?}")
        })?;
        EventKind::Stage { stage, status }
    };
    Ok(StoredEvent {
        id,
        at,
        event: Event {
            deploy_id,
            kind,
            detail,
        },
    })
}

/// Read a `(name, repo_path, route_domain, route_port)` row. The outer
/// `rusqlite::Result` is the column read; the inner `anyhow::Result` is the
/// route reconstruction, which can fail on a port outside `u16`.
fn app_config_row(row: &rusqlite::Row) -> rusqlite::Result<Result<AppConfig>> {
    Ok(build_app_config(
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
    ))
}

fn build_app_config(
    name: String,
    repo_path: String,
    domain: Option<String>,
    port: Option<i64>,
) -> Result<AppConfig> {
    // Both columns are written together, so one without the other means the
    // row was edited outside kuadrat. Refuse it rather than serve half a route.
    let route = match (domain, port) {
        (Some(domain), Some(port)) => {
            let parsed = u16::try_from(port).map_err(|_| {
                anyhow!("app {name:?} has route port {port}, which is outside 0-65535")
            })?;
            if parsed == 0 {
                bail!("app {name:?} has route port 0, which is not a valid port to serve on");
            }
            Some(Route {
                domain,
                port: parsed,
            })
        }
        (None, None) => None,
        (Some(_), None) => bail!("app {name:?} has a route domain but no port"),
        (None, Some(_)) => bail!("app {name:?} has a route port but no domain"),
    };
    Ok(AppConfig {
        name,
        repo_path,
        route,
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
                 WHERE type='table' AND name IN ('apps','deploys','locks','events','app_config')",
                [],
                |row| row.get(0),
            )
            .expect("query");
        assert_eq!(count, 5, "all five tables should exist");
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
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("first");
        store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Failed,
                Some("build broke".into()),
            ))
            .expect("second");

        let events = store.events_for(id).expect("read");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].event.kind,
            EventKind::Stage {
                stage: Stage::Detect,
                status: EventStatus::Started
            }
        );
        assert_eq!(
            events[1].event.kind,
            EventKind::Stage {
                stage: Stage::Build,
                status: EventStatus::Failed
            }
        );
        assert_eq!(events[1].event.detail.as_deref(), Some("build broke"));
    }

    #[test]
    fn events_are_scoped_to_their_deploy() {
        let (_dir, store) = open_temp();
        let a = store.create_deploy("a").expect("a");
        let b = store.create_deploy("b").expect("b");
        store
            .append_event(&Event::for_stage(
                a,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("append a");

        assert_eq!(store.events_for(a).expect("a").len(), 1);
        assert_eq!(store.events_for(b).expect("b").len(), 0);
    }

    /// The store fills in `at` on every read; no consumer can show per-stage
    /// timing if it comes back empty. Exact values are clock-dependent, so
    /// this only asserts non-emptiness and that ids still ascend.
    #[test]
    fn events_carry_a_non_empty_insert_timestamp() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");

        store
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("first");
        store
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Succeeded,
                None,
            ))
            .expect("second");

        let events = store.events_for(id).expect("read");
        assert_eq!(events.len(), 2);
        assert!(!events[0].at.is_empty(), "at must not be empty");
        assert!(!events[1].at.is_empty(), "at must not be empty");
        assert!(
            events[1].id > events[0].id,
            "ids must ascend: {:?}",
            (events[0].id, events[1].id)
        );
    }

    #[test]
    fn append_event_returns_the_assigned_id() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let id = store.create_deploy("web").unwrap();

        let first = store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Started,
                None,
            ))
            .expect("append")
            .id;
        let second = store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Succeeded,
                None,
            ))
            .expect("append")
            .id;

        assert!(first > 0, "id must be a real rowid, got {first}");
        assert!(
            second > first,
            "ids must increase: {second} came after {first}"
        );
    }

    /// The ids the stream replays from must be the ids the store handed out,
    /// or a reconnecting browser resumes from the wrong place.
    #[test]
    fn events_for_returns_the_same_ids_append_returned() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let id = store.create_deploy("web").unwrap();

        let mut appended = Vec::new();
        for status in [EventStatus::Started, EventStatus::Succeeded] {
            appended.push(
                store
                    .append_event(&Event::for_stage(id, Stage::Apply, status, None))
                    .expect("append")
                    .id,
            );
        }

        let read: Vec<i64> = store
            .events_for(id)
            .expect("read")
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(read, appended);
    }

    /// Ids are unique across deploys, not per-deploy — the SSE handler filters
    /// on `id > last_sent` and would drop events if two deploys reused ids.
    #[test]
    fn event_ids_are_unique_across_deploys() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let a = store.create_deploy("a").unwrap();
        let b = store.create_deploy("b").unwrap();

        let ev = |deploy_id| Event::for_stage(deploy_id, Stage::Detect, EventStatus::Started, None);
        let first = store.append_event(&ev(a)).expect("append").id;
        let second = store.append_event(&ev(b)).expect("append").id;

        assert_ne!(first, second);
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

    fn cfg(name: &str, repo: &str, route: Option<Route>) -> AppConfig {
        AppConfig {
            name: name.into(),
            repo_path: repo.into(),
            route,
        }
    }

    #[test]
    fn a_registered_app_reads_back_with_its_repo_path() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .register_app(&cfg("web", "/srv/web", None))
            .expect("register");

        let got = store.app_config("web").expect("read").expect("present");
        assert_eq!(got.name, "web");
        assert_eq!(got.repo_path, "/srv/web");
        assert_eq!(got.route, None);
    }

    #[test]
    fn registering_a_name_with_an_empty_slug_is_rejected() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let err = store
            .register_app(&cfg("@@@", "/srv/web", None))
            .expect_err("empty slug is rejected");
        assert!(err.to_string().contains("empty identifier"), "{err}");
    }

    #[test]
    fn registering_a_colliding_slug_is_rejected() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .register_app(&cfg("My App", "/srv/my-app", None))
            .expect("first registration");

        let err = store
            .register_app(&cfg("my_app", "/srv/other", None))
            .expect_err("colliding slug is rejected");
        assert!(
            err.to_string().contains("collides with an existing app"),
            "{err}"
        );
        assert!(err.to_string().contains("my-app"), "{err}");
    }

    /// A slug collision against a CLI-deployed `apps` row must also be
    /// rejected — registration is not the only writer of the identity space.
    #[test]
    fn registering_a_slug_that_collides_with_a_deployed_app_is_rejected() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .put_spec("my-app", "my-app", r#"{"name":"my-app"}"#)
            .expect("seed deployed app");

        let err = store
            .register_app(&cfg("My App", "/srv/web", None))
            .expect_err("colliding slug is rejected");
        assert!(
            err.to_string().contains("collides with an existing app"),
            "{err}"
        );
    }

    /// Re-registering the same name must not trip its own collision check.
    #[test]
    fn re_registering_the_same_name_still_succeeds() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .register_app(&cfg("web", "/srv/web", None))
            .expect("first registration");
        store
            .register_app(&cfg("web", "/srv/web-v2", None))
            .expect("re-registering the same app should succeed");

        let got = store.app_config("web").expect("read").expect("present");
        assert_eq!(got.repo_path, "/srv/web-v2");
    }

    #[test]
    fn registering_a_relative_repo_path_is_rejected() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        let err = store
            .register_app(&cfg("web", "myapp", None))
            .expect_err("relative repo_path is rejected");
        assert!(err.to_string().contains("not absolute"), "{err}");
    }

    #[test]
    fn a_route_survives_the_round_trip() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        let route = Route {
            domain: "example.com".into(),
            port: 3000,
        };

        store
            .register_app(&cfg("web", "/srv/web", Some(route.clone())))
            .expect("register");

        let got = store.app_config("web").expect("read").expect("present");
        assert_eq!(got.route, Some(route));
    }

    #[test]
    fn app_config_is_none_for_an_unregistered_app() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        assert_eq!(store.app_config("ghost").expect("read"), None);
    }

    /// Re-registering replaces the row rather than failing or duplicating —
    /// the UI's registration form is also how you correct a wrong path.
    #[test]
    fn registering_the_same_name_again_replaces_it() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .register_app(&cfg("web", "/srv/old", None))
            .expect("first");
        store
            .register_app(&cfg(
                "web",
                "/srv/new",
                Some(Route {
                    domain: "example.com".into(),
                    port: 8080,
                }),
            ))
            .expect("second");

        let got = store.app_config("web").expect("read").expect("present");
        assert_eq!(got.repo_path, "/srv/new");
        assert_eq!(got.route.expect("route").port, 8080);
    }

    /// Clearing a route must actually clear it. An upsert that only writes
    /// non-null values would leave the old route in place, and the app would
    /// keep serving on a domain the operator just removed.
    #[test]
    fn re_registering_without_a_route_clears_the_previous_one() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store
            .register_app(&cfg(
                "web",
                "/srv/web",
                Some(Route {
                    domain: "example.com".into(),
                    port: 3000,
                }),
            ))
            .expect("first");
        store
            .register_app(&cfg("web", "/srv/web", None))
            .expect("second");

        let got = store.app_config("web").expect("read").expect("present");
        assert_eq!(got.route, None, "route should have been cleared");
    }

    /// Opening the same file twice must not fail — `Store::open` runs the
    /// schema batch every time, and the acceptance host's database already
    /// exists.
    #[test]
    fn opening_an_existing_store_twice_succeeds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("k.db");

        let first = Store::open(&path).expect("first open");
        first
            .register_app(&cfg("web", "/srv/web", None))
            .expect("register");
        drop(first);

        let second = Store::open(&path).expect("second open");
        let got = second.app_config("web").expect("read").expect("present");
        assert_eq!(got.repo_path, "/srv/web");
    }

    /// The real migration case: a database created before this table existed
    /// must gain it on open, with its existing rows intact.
    #[test]
    fn a_pre_h2_database_gains_the_table_and_keeps_its_rows() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("k.db");

        // A database with only the pre-H2 tables, created without Store::open.
        {
            let conn = rusqlite::Connection::open(&path).expect("raw open");
            conn.execute_batch(
                "CREATE TABLE apps (
                     name       TEXT PRIMARY KEY,
                     slug       TEXT NOT NULL UNIQUE,
                     spec_json  TEXT NOT NULL,
                     updated_at TEXT NOT NULL DEFAULT (datetime('now'))
                 );",
            )
            .expect("old schema");
            conn.execute(
                "INSERT INTO apps (name, slug, spec_json) VALUES ('legacy', 'legacy', '{}')",
                [],
            )
            .expect("seed");
        }

        let store = Store::open(&path).expect("open upgrades");

        // The new table works...
        store
            .register_app(&cfg("legacy", "/srv/legacy", None))
            .expect("register");
        assert_eq!(
            store
                .app_config("legacy")
                .expect("read")
                .expect("present")
                .repo_path,
            "/srv/legacy"
        );
        // ...and the pre-existing row was not disturbed.
        assert_eq!(
            store.current_spec("legacy").expect("spec").as_deref(),
            Some("{}")
        );
    }

    #[test]
    fn listing_returns_every_registration_ordered_by_name() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();

        store.register_app(&cfg("web", "/srv/web", None)).unwrap();
        store.register_app(&cfg("api", "/srv/api", None)).unwrap();
        store.register_app(&cfg("jobs", "/srv/jobs", None)).unwrap();

        let names: Vec<String> = store
            .list_app_configs()
            .expect("list")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["api", "jobs", "web"]);
    }

    #[test]
    fn listing_an_empty_store_is_empty_not_an_error() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        assert!(store.list_app_configs().expect("list").is_empty());
    }

    /// A listed registration carries its route, so the app list can show the
    /// domain without a second query per row.
    #[test]
    fn a_listed_registration_carries_its_route() {
        let dir = tempdir().unwrap();
        let store = Store::open(&dir.path().join("k.db")).unwrap();
        store
            .register_app(&cfg(
                "web",
                "/srv/web",
                Some(Route {
                    domain: "example.com".into(),
                    port: 3000,
                }),
            ))
            .unwrap();

        let all = store.list_app_configs().expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].route.as_ref().expect("route").domain, "example.com");
    }

    /// Port 0 cannot reach `register_app` through the `Route`/`u16` type
    /// today, but a hand-edited row could carry it — reading it back must
    /// refuse it rather than construct a `Route` nothing can serve on.
    #[test]
    fn reading_a_route_with_port_zero_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("k.db");
        let store = Store::open(&path).expect("open");
        store
            .register_app(&cfg("web", "/srv/web", None))
            .expect("register");
        {
            let conn = store.conn.lock().expect("lock");
            conn.execute(
                "UPDATE app_config SET route_domain = 'example.com', route_port = 0
                 WHERE name = 'web'",
                [],
            )
            .expect("seed invalid port");
        }

        let err = store.app_config("web").expect_err("port 0 is rejected");
        assert!(err.to_string().contains("port 0"), "{err}");
        assert!(err.to_string().contains("not a valid port"), "{err}");
    }

    #[test]
    fn a_deploy_level_event_round_trips_through_the_stage_column() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");
        store
            .append_event(&Event::finished(
                id,
                DeployStatus::RolledBack,
                Some("apply broke".into()),
            ))
            .expect("append");

        let events = store.events_for(id).expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].event.kind,
            EventKind::Finished {
                status: DeployStatus::RolledBack
            }
        );
        assert_eq!(events[0].event.detail.as_deref(), Some("apply broke"));
    }

    /// The literal that separates the two kinds is a storage detail, but a
    /// wrong one is silent: a stage named "deploy" would read back as a
    /// terminal event. Pin the spelling.
    #[test]
    fn a_deploy_level_event_is_stored_under_the_literal_deploy() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");
        store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");

        let conn = store.conn.lock().expect("store lock");
        let stage: String = conn
            .query_row(
                "SELECT stage FROM events WHERE deploy_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .expect("query");
        assert_eq!(stage, "deploy");
    }

    /// `in_progress` is a `DeployStatus` but not a terminal one. A row saying
    /// the deploy finished in progress is corrupt, and must not read back as
    /// a valid event.
    #[test]
    fn a_deploy_level_row_that_is_not_terminal_is_an_error() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");
        {
            let conn = store.conn.lock().expect("store lock");
            conn.execute(
                "INSERT INTO events (deploy_id, stage, status) VALUES (?1, 'deploy', 'in_progress')",
                params![id],
            )
            .expect("insert");
        }
        let err = store.events_for(id).unwrap_err();
        assert!(err.to_string().contains("not terminal"), "was: {err}");
    }

    /// The property Finding 1 closes: a reader that sees the row terminal
    /// also sees the terminal event, because both writes land in one
    /// transaction. This only proves the "commit" half — that after a
    /// successful call, the row and the event agree — since provoking the
    /// append to fail mid-transaction (to prove the rollback half) is not
    /// reachable from this test: nothing here can make the `INSERT INTO
    /// events` half of `finish_deploy_with_event` fail independently of the
    /// `UPDATE deploys` half using only the public `Store` API.
    #[test]
    fn finish_deploy_with_event_leaves_the_row_and_the_event_in_agreement() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");

        let stored = store
            .finish_deploy_with_event(id, DeployStatus::Done, Some("all stages ok"))
            .expect("finish with event");

        let row = store.deploy(id).expect("get").expect("row");
        assert_eq!(row.status, DeployStatus::Done);
        assert_eq!(row.detail.as_deref(), Some("all stages ok"));

        let events = store.events_for(id).expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, stored.id);
        assert_eq!(
            events[0].event.kind,
            EventKind::Finished {
                status: DeployStatus::Done
            }
        );
        assert_eq!(events[0].event.detail.as_deref(), Some("all stages ok"));
    }

    /// A deploy id that does not exist must fail before either write commits
    /// — no orphan event for a row that was never touched.
    #[test]
    fn finish_deploy_with_event_on_an_unknown_id_writes_nothing() {
        let (_dir, store) = open_temp();
        let err = store
            .finish_deploy_with_event(999, DeployStatus::Done, None)
            .unwrap_err();
        assert!(err.to_string().contains("999"), "{err}");
        assert_eq!(
            store.events_for(999).expect("read").len(),
            0,
            "no event should have been left behind by the failed call"
        );
    }

    #[test]
    fn a_stage_event_still_round_trips_unchanged() {
        let (_dir, store) = open_temp();
        let id = store.create_deploy("web").expect("create");
        store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Failed,
                None,
            ))
            .expect("append");

        let events = store.events_for(id).expect("read");
        assert_eq!(
            events[0].event.kind,
            EventKind::Stage {
                stage: Stage::Build,
                status: EventStatus::Failed
            }
        );
    }

    #[test]
    fn recent_deploys_returns_an_apps_history_newest_first() {
        let (_dir, store) = open_temp();
        let first = store.create_deploy("web").expect("first");
        let second = store.create_deploy("web").expect("second");

        let rows = store.recent_deploys("web", 10).expect("read");
        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![second, first]
        );
    }

    #[test]
    fn recent_deploys_is_scoped_to_one_app() {
        let (_dir, store) = open_temp();
        store.create_deploy("web").expect("web");
        let api = store.create_deploy("api").expect("api");

        let rows = store.recent_deploys("api", 10).expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, api);
    }

    #[test]
    fn recent_deploys_honours_its_limit() {
        let (_dir, store) = open_temp();
        for _ in 0..5 {
            store.create_deploy("web").expect("create");
        }
        assert_eq!(store.recent_deploys("web", 2).expect("read").len(), 2);
    }

    /// An app that has never deployed is an empty history, not an error — the
    /// page renders "no deploys yet" and must not 500.
    #[test]
    fn an_app_with_no_deploys_has_an_empty_history() {
        let (_dir, store) = open_temp();
        assert!(store
            .recent_deploys("nothing", 10)
            .expect("read")
            .is_empty());
    }

    /// `usize::MAX` is what a caller writes when it means "no ceiling". It must
    /// not reach SQLite as a negative LIMIT, which would mean the same thing by
    /// accident — and would be the one call that escapes the bound.
    ///
    /// Note: this test cannot prove the clamp itself, because a negative LIMIT
    /// returns everything and there are only three rows. It proves the call is
    /// well-formed and does not error when given an enormous limit.
    #[test]
    fn an_enormous_limit_is_clamped_rather_than_wrapping_negative() {
        let (_dir, store) = open_temp();
        for _ in 0..3 {
            store.create_deploy("web").expect("create");
        }
        assert_eq!(
            store.recent_deploys("web", usize::MAX).expect("read").len(),
            3
        );
    }
}
