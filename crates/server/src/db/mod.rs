use anyhow::Result;
use rusqlite::Connection;
use std::sync::Mutex;

pub mod models;

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        // Performance tuning for minimal overhead
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -2000;
             PRAGMA foreign_keys = ON;"
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS repositories (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                owner       TEXT NOT NULL,
                url         TEXT NOT NULL,
                description TEXT,
                webhook_url TEXT,
                filesystem_path TEXT,
                ssh_password TEXT,
                is_private  INTEGER NOT NULL DEFAULT 0,
                default_branch TEXT NOT NULL DEFAULT 'main',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS deployments (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id     INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
                status      TEXT NOT NULL DEFAULT 'pending',
                commit_sha  TEXT,
                image_name  TEXT,
                container_id TEXT,
                domain      TEXT,
                port        INTEGER,
                build_log   TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                job_type    TEXT NOT NULL,
                payload     TEXT NOT NULL DEFAULT '{}',
                status      TEXT NOT NULL DEFAULT 'pending',
                result      TEXT,
                attempts    INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
            CREATE INDEX IF NOT EXISTS idx_deployments_repo ON deployments(repo_id);
            "
        )?;

        // Idempotent column additions — errors are silently ignored when the
        // column already exists (SQLite returns "duplicate column name").
        let _ = conn.execute("ALTER TABLE repositories ADD COLUMN domain TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE repositories ADD COLUMN proxy_port INTEGER DEFAULT 3000",
            [],
        );

        // Environment variables per repository
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS repo_env_vars (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                repo_id     INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
                key         TEXT NOT NULL,
                value       TEXT NOT NULL DEFAULT '',
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(repo_id, key)
            );
            CREATE INDEX IF NOT EXISTS idx_env_vars_repo ON repo_env_vars(repo_id);",
        )?;

        tracing::info!("Database migrations complete");
        Ok(())
    }

    /// Execute a closure with exclusive access to the database connection.
    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}
