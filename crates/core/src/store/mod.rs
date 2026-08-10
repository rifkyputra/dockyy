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
    // Read only by tests today; Tasks 4-7 add the methods that use it for
    // real. `#[allow(dead_code)]` avoids a clippy `-D warnings` failure on
    // the non-test build in the meantime.
    #[allow(dead_code)]
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
