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

            CREATE TABLE IF NOT EXISTS login_attempts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                ip_address  TEXT NOT NULL,
                success     INTEGER NOT NULL DEFAULT 0,
                attempted_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_login_attempts_ip ON login_attempts(ip_address, attempted_at);
            "
        )?;

        // Idempotent column additions — errors are silently ignored when the
        // column already exists (SQLite returns "duplicate column name").
        let _ = conn.execute("ALTER TABLE repositories ADD COLUMN domain TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE repositories ADD COLUMN proxy_port INTEGER DEFAULT 3000",
            [],
        );

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

    /// Record a login attempt (success or failure) for the given IP.
    pub fn record_login_attempt(&self, ip: &str, success: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO login_attempts (ip_address, success) VALUES (?1, ?2)",
            rusqlite::params![ip, success as i32],
        )?;
        // On successful login, clear previous failed attempts for this IP
        if success {
            conn.execute(
                "DELETE FROM login_attempts WHERE ip_address = ?1 AND success = 0",
                rusqlite::params![ip],
            )?;
        }
        Ok(())
    }

    /// Count consecutive failed login attempts for an IP since the last success
    /// and return how many seconds the IP must still wait (0 = allowed).
    ///
    /// Lockout rule: after `n` consecutive failures (max 3 before lockout),
    /// the IP must wait `3 minutes * n` since the last failure.
    pub fn check_login_rate_limit(&self, ip: &str) -> Result<(u32, i64)> {
        let conn = self.conn.lock().unwrap();

        // Count consecutive failures (no successful login in between)
        let failed_count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM login_attempts
             WHERE ip_address = ?1 AND success = 0
             AND attempted_at > COALESCE(
                 (SELECT MAX(attempted_at) FROM login_attempts
                  WHERE ip_address = ?1 AND success = 1),
                 '1970-01-01'
             )",
            rusqlite::params![ip],
            |row| row.get(0),
        )?;

        if failed_count < 3 {
            return Ok((failed_count, 0));
        }

        // Get time of last failed attempt
        let last_attempt: String = conn.query_row(
            "SELECT MAX(attempted_at) FROM login_attempts
             WHERE ip_address = ?1 AND success = 0",
            rusqlite::params![ip],
            |row| row.get(0),
        )?;

        let last_attempt_ts = chrono::NaiveDateTime::parse_from_str(&last_attempt, "%Y-%m-%d %H:%M:%S")
            .unwrap_or_default();
        let now = chrono::Utc::now().naive_utc();
        let lockout_seconds = (failed_count as i64) * 180; // 3 minutes * attempt count
        let elapsed = (now - last_attempt_ts).num_seconds();
        let remaining = lockout_seconds - elapsed;

        Ok((failed_count, remaining.max(0)))
    }
}
