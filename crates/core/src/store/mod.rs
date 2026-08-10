//! SQLite-backed durable state for kuadrat: specs, deploy history, the durable
//! stage, per-app locks, and the event log.
//!
//! The store opens its own SQLite file directly, NOT through the `FileSystem`
//! seam. This is deliberate and sanctioned (ADR-0002): the database is
//! kuadrat's own state, not a side effect on a managed host. A future remote
//! executor keeps its store wherever kuadrat runs.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

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
}
