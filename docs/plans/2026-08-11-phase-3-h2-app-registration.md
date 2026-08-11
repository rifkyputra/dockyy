# kuadrat Phase 3 · H2 — App Registration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** kuadrat remembers where an app's source lives, so it can be redeployed by something that
has no command line.

**Architecture:** A new `app_config` table holding what the operator asked for — name, repo path,
optional route — beside the existing `apps` table holding what was actually deployed. Two tables
because they have different lifetimes: a registration exists before the first deploy, a deployed
spec only after it.

**Tech Stack:** Rust 2021, `rusqlite` (bundled, SQLite 3.46), `anyhow`, `tempfile`.

**Design:** [`docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`](../design/2026-08-11-phase-3-daemon-and-surfaces.md)

## Global Constraints

- **`core` never opens a socket and never takes a `host` parameter.** No HTTP or transport
  dependency enters `crates/core`.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
  Run `cargo fmt` before every commit.
- **`cargo test --all` must pass.** Baseline at `9a5a3df` is **140 total** — `kuadrat-core` 136 plus
  `kuadrat` (cli) 4, printed as two separate `test result:` lines. Counts below give both, because
  the core figure alone looks like a total and will not match what the command prints.
- **Prefix cargo commands with `PATH=$HOME/.cargo/bin:$PATH`.** Otherwise `cargo` is not on PATH.
- **No new dependencies.**
- **Secret values never appear** in the store, logs, error messages, or committed files. A repo path
  is not a secret; a URL with an embedded token would be — see Task 1 Step 7.
- **An existing database must keep working.** The acceptance host has one. Every schema change must
  be a no-op against a database that already exists.
- Commit after every task with a Conventional Commit subject.

## A correction to the design document

The design says the `apps` table "gains `repo_path TEXT` and `route TEXT`" via an idempotent
`ALTER TABLE`. **That cannot work**, and Task 3 amends the design.

`apps` is `name TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, spec_json TEXT NOT NULL`. Registration
happens *before* an app's first deploy — that is its entire purpose, since a browser has no argv to
supply a repo path from. At that moment there is no spec and no slug, so the row cannot be inserted:
`spec_json` is `NOT NULL` with no default. Adding nullable columns does not help, because the
blocker is a constraint on an existing column.

SQLite cannot drop `NOT NULL` with `ALTER TABLE`; removing it means create-copy-drop-rename on the
one table holding user data. A sentinel (`spec_json = ''`) avoids the rebuild but makes
`current_spec` return `Some("")`, so `resolve_spec` fails with `EOF while parsing a value` instead
of its clean "no spec for `<app>`" error — a hack that leaks into an unrelated code path.

A separate table avoids all of it. `CREATE TABLE IF NOT EXISTS app_config (…)` added to the existing
`SCHEMA` batch is **automatically correct on an existing database** — that is exactly what
`IF NOT EXISTS` is for — so there is no `ALTER TABLE`, no idempotency question, and no migration to
get wrong. The two tables also mean different things and are honest about it:

| Table | Meaning | Written by |
|---|---|---|
| `apps` | what was actually deployed | the deploy loop, on success |
| `app_config` | what the operator asked for | registration |

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/core/src/store/mod.rs` | *Modify.* `app_config` table in `SCHEMA`; `AppConfig` struct; `register_app`, `app_config`, `list_app_configs` |

One file. `AppConfig` lives beside `DeployRow` in `store/mod.rs` because it is a row type, and
`DeployRow` establishes that pattern.

---

### Task 1: The `app_config` table, `register_app`, and reading one back

**Files:**
- Modify: `crates/core/src/store/mod.rs` (the `SCHEMA` constant ~line 21; a new struct beside
  `DeployRow` ~line 53; new methods on `impl Store`; new tests in the `mod tests` block)

**Interfaces:**
- Consumes: `crate::spec::Route` — `pub struct Route { pub domain: String, pub port: u16 }`
- Produces:
  - `pub struct AppConfig { pub name: String, pub repo_path: String, pub route: Option<Route> }`
  - `Store::register_app(&self, config: &AppConfig) -> Result<()>` — upsert by name
  - `Store::app_config(&self, name: &str) -> Result<Option<AppConfig>>`

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block at the bottom of `crates/core/src/store/mod.rs`. The module already has
`use super::*;` and `use tempfile::tempdir;`.

```rust
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
            store.app_config("legacy").expect("read").expect("present").repo_path,
            "/srv/legacy"
        );
        // ...and the pre-existing row was not disturbed.
        assert_eq!(
            store.current_spec("legacy").expect("spec").as_deref(),
            Some("{}")
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test --all app_config`
Expected: FAIL to compile — `cannot find type AppConfig`, `no method named register_app`.

- [ ] **Step 3: Add the table to the schema**

In `crates/core/src/store/mod.rs`, append to the `SCHEMA` constant, after the `events` table and
before the closing `";`:

```sql
CREATE TABLE IF NOT EXISTS app_config (
    name         TEXT PRIMARY KEY,
    repo_path    TEXT NOT NULL,
    route_domain TEXT,
    route_port   INTEGER,
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

The route is two nullable columns rather than one JSON blob so that a future query can filter on the
domain without parsing. Both are null together or set together; Step 5 enforces that on read.

`CREATE TABLE IF NOT EXISTS` inside the existing batch is the whole migration. On a database that
predates this table, the statement creates it; on one that has it, the statement does nothing. There
is no `ALTER TABLE` and therefore no idempotency question.

- [ ] **Step 4: Add the `AppConfig` type**

Beside `DeployRow` (~line 53):

```rust
/// What the operator asked for: where an app's source lives, and optionally
/// the domain it should be served on.
///
/// Distinct from the `apps` row, which records what was actually deployed. A
/// registration exists from the moment someone adds the app; a deployed spec
/// only exists after a deploy succeeds. Keeping them apart is what lets an app
/// be registered before it has ever been built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub name: String,
    pub repo_path: String,
    pub route: Option<Route>,
}
```

`Route` is **not** currently imported in this file — verified at `9a5a3df`, where the imports are
`use anyhow::{anyhow, bail, Context, Result};` and
`use rusqlite::{params, Connection, OptionalExtension};`. Add the line:

```rust
use crate::spec::Route;
```

`anyhow!` and `bail!`, used in Step 5, are already imported — do not add them again.

- [ ] **Step 5: Add `register_app` and `app_config`**

On `impl Store`, beside the other methods:

```rust
    /// Register an app, or replace an existing registration.
    ///
    /// The upsert writes every column unconditionally, including the route
    /// columns when the route is `None`. Writing only the non-null values
    /// would make clearing a route impossible — the app would keep serving on
    /// a domain the operator had just removed.
    pub fn register_app(&self, config: &AppConfig) -> Result<()> {
        let (domain, port) = match &config.route {
            Some(route) => (Some(route.domain.as_str()), Some(route.port as i64)),
            None => (None, None),
        };
        let conn = self.conn.lock().expect("store lock");
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
```

And the row mapper, beside `deploy_row` and `event_row` near the bottom of the file:

```rust
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
            let port = u16::try_from(port).map_err(|_| {
                anyhow!("app {name:?} has route port {port}, which is outside 1-65535")
            })?;
            Some(Route { domain, port })
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
```

`bail!`, `anyhow!`, `params!` and `OptionalExtension` (for `.optional()`) are all already imported
at the top of this file. The only import you add in this task is `use crate::spec::Route;` from
Step 4.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test --all`
Expected: PASS. Core **143**, cli 4 — **147 total**.

- [ ] **Step 7: Confirm no secret can be written here**

Run: `grep -n "repo_path" crates/core/src/store/mod.rs`

`repo_path` is a local filesystem path — `/srv/web`, not a URL. kuadrat never clones, so no
credential is ever part of it (that decision is recorded in the phase 2 design under "Out: cloning
from a git URL, and therefore all git credential handling"). Nothing in this task should accept or
store a URL. If you find yourself adding one, stop — that is a scope change with a security
dimension, and it belongs to a different plan.

- [ ] **Step 8: Run the full gate**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo fmt && cargo test --all && cargo clippy --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 9: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): register an app's source path and route

A browser has no argv, so redeploying from the UI needs the repo path to
be remembered. app_config holds what the operator asked for, beside apps
which holds what was actually deployed — two tables because they have
different lifetimes: a registration exists before the first deploy, a
deployed spec only after it.

A new CREATE TABLE IF NOT EXISTS is the entire migration, which is why
this is not the ALTER TABLE the design called for: apps.spec_json is NOT
NULL, so a pre-deploy row cannot be inserted there at all, and SQLite
cannot drop NOT NULL without rebuilding the table that holds user data.

The upsert writes the route columns even when the route is None, or
clearing a route would be impossible and the app would keep serving on a
domain the operator had just removed."
```

---

### Task 2: Listing registrations

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Consumes, all from Task 1 and already present in the same test module:
  - `AppConfig { name, repo_path, route }`
  - `app_config_row` — the row mapper, reused unchanged
  - the test helper `fn cfg(name: &str, repo: &str, route: Option<Route>) -> AppConfig`; the tests
    below call it, so do not redefine it
- Produces: `Store::list_app_configs(&self) -> Result<Vec<AppConfig>>` — ordered by name

The daemon's `GET /` renders every registered app. Without this it would have to know the names in
advance, which is the problem registration exists to solve.

- [ ] **Step 1: Write the failing tests**

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test --all list_app_configs`
Expected: FAIL to compile — `no method named list_app_configs`.

- [ ] **Step 3: Implement it**

On `impl Store`, directly after `app_config`:

```rust
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
```

The `??` is the same two-layer unwrap `events_for` uses: the first `?` is the column read, the
second is the route reconstruction.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test --all`
Expected: PASS. Core **146**, cli 4 — **150 total**.

- [ ] **Step 5: Run the full gate**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo fmt && cargo test --all && cargo clippy --all-targets -- -D warnings`
Expected: all clean.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): list registered apps for the daemon's app list

Ordered in SQL so the list is stable between requests, and each row
carries its route so rendering the list needs no second query."
```

---

### Task 3: Amend the design document

**Files:**
- Modify: `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`

**Interfaces:**
- Consumes: everything from Tasks 1-2. No code changes.

The design describes a migration that cannot work. Left as written, whoever builds H4 will try to
`ALTER TABLE apps`, hit the `NOT NULL` wall, and have to re-derive this reasoning.

- [ ] **Step 1: Replace the `store` bullet under "What changes in `core`"**

Find the bullet reading:

> - `store`: `apps` gains `repo_path TEXT` and `route TEXT`, both nullable, with
>   `register_app`/`app_row` accessors. See Migration below.

Replace with:

```markdown
- `store`: a new `app_config` table — `name`, `repo_path`, `route_domain`, `route_port` — with
  `register_app`, `app_config` and `list_app_configs` accessors. **Not** new columns on `apps`: see
  Registration storage below.
```

- [ ] **Step 2: Replace the "Migration" section**

Find the `### Migration` heading and replace the whole section — heading and body, through to the
next `##` heading — with:

```markdown
### Registration storage

Registration does **not** extend the `apps` table, and there is no `ALTER TABLE`.

`apps` is `name TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, spec_json TEXT NOT NULL`. A
registration exists *before* an app's first deploy — that is its purpose, since a browser has no
argv to supply a repo path from — so at registration time there is no spec and no slug, and the row
cannot be inserted at all. Nullable new columns do not help: the blocker is a `NOT NULL` on an
existing column, and SQLite cannot drop one without rebuilding the table that holds user data.

A separate table sidesteps it entirely:

```sql
CREATE TABLE IF NOT EXISTS app_config (
    name         TEXT PRIMARY KEY,
    repo_path    TEXT NOT NULL,
    route_domain TEXT,
    route_port   INTEGER,
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Added to the existing `SCHEMA` batch, this is correct on an existing database by construction —
`IF NOT EXISTS` creates it where it is missing and does nothing where it is not. No migration step,
no idempotency question.

The split is also honest about meaning: `apps` records **what was deployed**, written by the deploy
loop on success; `app_config` records **what the operator asked for**, written by registration. They
have different lifetimes and different writers.

The route is two nullable columns rather than one blob so a query can filter on domain without
parsing. They are written together and read together — one without the other means the row was
edited outside kuadrat, and the read refuses it rather than serving half a route. The upsert writes
both columns unconditionally, including when the route is `None`: an upsert that skipped nulls would
make clearing a route impossible, leaving an app served on a domain the operator had just removed.
```

- [ ] **Step 3: Verify the document has no other stale claim about this**

Run: `grep -n "ALTER TABLE\|repo_path\|app_row\|register_app" docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`

Every hit must describe the `app_config` table. `app_row` was the design's old name for the reader
and no longer exists — the accessors are `app_config` and `list_app_configs`. If any hit still
refers to altering `apps` or to `app_row`, fix it.

- [ ] **Step 4: Verify every named method exists**

Run: `grep -n "pub fn register_app\|pub fn app_config\|pub fn list_app_configs" crates/core/src/store/mod.rs`

All three must be present. A design document naming a method that does not exist is the drift this
step exists to prevent.

- [ ] **Step 5: Commit**

```bash
git add docs/design/2026-08-11-phase-3-daemon-and-surfaces.md
git commit -m "docs(design): registration gets its own table, not new apps columns

The design specified an idempotent ALTER TABLE adding repo_path and
route to apps. That cannot work: registration happens before the first
deploy, apps.spec_json is NOT NULL, and SQLite cannot drop NOT NULL
without rebuilding the table holding user data.

A separate app_config table needs no migration at all — CREATE TABLE IF
NOT EXISTS in the existing schema batch is correct on an existing
database by construction."
```

---

## H2 completion checklist

- [ ] `cargo test --all` passes: core **146** (136 + 10 new), cli 4 — **150 total**
- [ ] `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean
- [ ] `app_config` table created via `CREATE TABLE IF NOT EXISTS` in the existing `SCHEMA` batch
- [ ] **No `ALTER TABLE` anywhere** — `grep -rn "ALTER TABLE" crates/` returns nothing
- [ ] A database created with the pre-H2 schema opens, gains the table, and keeps its rows
- [ ] Opening the same store twice succeeds
- [ ] Re-registering without a route clears the previous route
- [ ] A route round-trips through `register_app` → `app_config` unchanged
- [ ] The design document describes `app_config`, and names no method that does not exist
- [ ] No new dependency in any `Cargo.toml`

## Not in H2 (later groups)

| Group | What |
|---|---|
| H3 | The `logs` module — `tail`, `search` |
| H4 | `crates/daemon`, config, loopback guard, router, JSON API, the global semaphore. **`POST /api/apps` calls `register_app`; the deploy handler reads `repo_path` from `app_config`** |
| H5 | `BroadcastSink`, the SSE hub, backlog-then-live, dedupe, lag recovery, `Last-Event-ID` |
| H6 | The three htmx pages, embedded assets |
| H7 | Webhook sender, `kuadrat serve`, the systemd unit, `kuadrat deploy` as a client |

Nothing in H2 is wired to a caller. `deploy::run` still takes its repo path as an argument, and the
CLI still reads it from argv. H4 is where registration starts being used — that is deliberate, so
this group can be reviewed as a storage change on its own.
