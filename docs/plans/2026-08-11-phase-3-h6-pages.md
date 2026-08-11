# kuadrat Phase 3 · H6 — The Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** An operator opens `http://127.0.0.1:7457/`, sees every registered app and its live status,
registers a new one, clicks into it, presses redeploy, and watches the six stages arrive one by one
without touching a terminal.

**Architecture:** Four new daemon modules. `error.rs` holds the API error type so it can be shared.
`stream.rs` takes the SSE ordering machinery out of `api.rs` and parameterises it by a renderer, so
the JSON stream and the new HTML stream share one implementation of the part that is hard to get
right. `pages.rs` renders with `maud` and holds the page handlers. `assets.rs` serves the embedded
htmx, its SSE extension, and the stylesheet.

**Tech Stack:** Rust 2021, axum 0.7, `maud` 0.26 (`axum` feature), htmx 2.0.10 + `htmx-ext-sse`
2.2.4 (vendored), `async-stream`, `rusqlite`.

**Design:** [`docs/design/2026-08-11-phase-3-h6-pages.md`](../design/2026-08-11-phase-3-h6-pages.md),
which in turn refines [`2026-08-11-phase-3-daemon-and-surfaces.md`](../design/2026-08-11-phase-3-daemon-and-surfaces.md).

## Global Constraints

- **`core` never opens a socket and never takes a `host` parameter** (ADR-0002). `core` gains one
  read method in this group (Task 4) and nothing else.
- **Escaping is not optional.** These pages interpolate journald content, which `known-gaps.md`
  records as "whatever the application wrote to its stdout and stderr". `maud` escapes by default;
  **`maud::PreEscaped` must not appear anywhere in this group.** If a task seems to need it, stop
  and report rather than reaching for it.
- **The eight existing JSON stream tests in `api.rs` must pass unchanged** through Task 1's
  extraction. They are the evidence the refactor preserved behaviour. Changing one to accommodate
  the refactor defeats its purpose — if one must change, stop and report why.
- **Subscribe before any read** stays the stream's load-bearing ordering, and its doc comment stays
  with the code that owns it. Moving the machinery must not reorder it.
- **`make check` must pass**: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
  Run `cargo fmt` before every commit.
- **Prefix cargo commands with `PATH=$HOME/.cargo/bin:$PATH`.**
- **Baselines, measured at `5217477`:** `kuadrat` (cli) **17**, `kuadrat_core` **182**,
  `kuadrat_daemon` **38**.
- **New dependencies, daemon-only:** `maud = { version = "0.26", features = ["axum"] }`. Nothing
  else, and nothing new in `core`. **Not 0.27** — its `axum` feature depends on `axum-core 0.5`
  (axum 0.8), while this workspace is on axum 0.7 / axum-core 0.4, and the build fails. 0.26 is the
  version whose `axum` feature matches axum 0.7.
- **No secret values** in logs, error messages, or committed files.
- Commit after every task with a Conventional Commit subject.

## Two things the design document does not say

**1. `/app/:name` needs a store method that does not exist.** The page shows an app's recent
deploys. `Store` has `deploy(id)` and `in_progress_deploys()`, but nothing that lists an app's
history. Task 4 adds `recent_deploys(app, limit)`. It is a read, it goes through no new seam, and it
is the only `core` change in this group.

**2. Content negotiation defaults to JSON, not to the redirect.** The parent design says the deploy
route returns "`303` (browser) / `200 {deploy_id}` (JSON)" without saying which way an ambiguous
request falls. This plan sends the `303` **only when `Accept` contains `text/html`**, and JSON
otherwise.

That direction is the safe one. Browsers always send `text/html` in `Accept`, so they get the
redirect either way; an API client that forgets the header gets JSON rather than a redirect it did
not expect. Defaulting the other way would turn every existing `curl` and every existing test that
posts without an `Accept` header into a redirect follower, silently.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/daemon/src/error.rs` | *Create.* `ApiError`, `ApiResult` — moved out of `api.rs` so `stream.rs` can use them without a module cycle |
| `crates/daemon/src/stream.rs` | *Create.* The SSE engine: ordering, dedupe, lag recovery, termination, the 204 rule. Renderer-agnostic |
| `crates/daemon/src/assets.rs` | *Create.* Embedded htmx, SSE extension, stylesheet; their content types |
| `crates/daemon/assets/*` | *Create.* The vendored files themselves, with provenance |
| `crates/daemon/src/pages.rs` | *Create.* maud rendering and the page handlers |
| `crates/daemon/src/api.rs` | *Modify.* Loses the stream machinery; gains content negotiation |
| `crates/daemon/src/lib.rs` | *Modify.* Declare the new modules |
| `crates/core/src/store/mod.rs` | *Modify.* `recent_deploys` |

`pages.rs` will be the biggest new file. If it passes roughly 600 lines including tests, split the
rendering functions into `pages/render.rs` and keep the handlers in `pages/mod.rs` — but only then,
and say so in the report rather than deciding silently.

---

### Task 1: Extract the stream engine

**Files:**
- Create: `crates/daemon/src/error.rs`, `crates/daemon/src/stream.rs`
- Modify: `crates/daemon/src/api.rs`, `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `AppState { hub, store, .. }`, `Store::deploy`, `Store::events_for`, `StoredEvent`,
  `EventKind`, `BroadcastSink::subscribe`
- Produces:
  - `pub struct ApiError(StatusCode, String)` with `not_found`/`bad_request`/`conflict`/`internal`,
    and `pub type ApiResult<T> = Result<T, ApiError>` — both in `error.rs`
  - `pub fn resume_from(headers: &HeaderMap) -> i64` (moved to `stream.rs`)
  - `pub fn events_sse<F>(st: &AppState, id: i64, headers: &HeaderMap, render: F) -> ApiResult<Response>`
    where `F: Fn(&StoredEvent) -> sse::Event + Send + 'static`

This task is a pure move. **No behaviour changes at all** — the 204 rule is Task 2's job.

- [ ] **Step 1: Move the error type**

Create `crates/daemon/src/error.rs` holding `ApiError`, its four constructors, its `IntoResponse`
impl, and `ApiResult`, exactly as they are in `api.rs` today. Make `ApiResult` `pub`. Keep the
existing doc comment — it explains why the status is chosen at the point the condition is detected.

In `api.rs`, replace the definitions with `use crate::error::{ApiError, ApiResult};`.

Declare `pub mod error;` in `lib.rs`.

- [ ] **Step 2: Run the suite — nothing should have changed**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep "test result"
```

Expected: cli 17, core 182, daemon 38 — unchanged. A move that changes a count is not a move.

- [ ] **Step 3: Move the stream machinery**

Create `crates/daemon/src/stream.rs`. Move into it, unchanged except where noted: `deploy_events`'s
body, `is_finished`, and `resume_from`. Leave `sse_event` (the JSON renderer) and `EventOut` in
`api.rs` — that is the part that differs per caller.

The module doc:

```rust
//! The SSE engine: everything about *when* an event reaches a subscriber, and
//! nothing about what it looks like when it gets there.
//!
//! Two handlers need this: the JSON API stream and the page's HTML stream.
//! They differ only in how one event is rendered, which is three lines. What
//! they share — subscribe before any read, send the backlog, forward live
//! events with `id > last_sent`, recover from a lag by re-reading SQLite,
//! close when the deploy ends — is the part that is hard to get right, cost a
//! fix round in H5 and another after the whole-branch review, and cannot be
//! fully covered by tests: no `.await` point exists between the subscribe and
//! the backlog read, so no test can tell the correct ordering from the
//! reversed one. A second copy of that would be a second place for it to rot
//! silently.
```

The function:

```rust
/// One deploy's events as an SSE response, rendered by `render`.
///
/// **Subscribing happens before every read**, the deploy row included. An
/// event landing between the backlog read and the subscription would be lost
/// permanently, and it is precisely the stage transition the viewer is waiting
/// for; in this order it arrives twice instead, and the `id > last_sent`
/// filter drops the duplicate. A duplicate is recoverable; a gap is not. The
/// row read carries no ordering requirement of its own and sits behind the
/// subscription only so that nobody has to reason about which reads count.
pub fn events_sse<F>(
    st: &AppState,
    id: i64,
    headers: &HeaderMap,
    render: F,
) -> ApiResult<Response>
where
    F: Fn(&StoredEvent) -> sse::Event + Send + 'static,
{
    // ... the existing body, with `sse_event(&ev)` replaced by `render(&ev)`
    // and the tail becoming:
    //     Ok(Sse::new(stream).keep_alive(KeepAlive::default()).into_response())
}
```

The return type becomes `Response` rather than `Sse<impl Stream<..>>` because Task 2 adds a second
possible response. Doing it now keeps Task 2 from touching this signature.

`api.rs`'s handler shrinks to:

```rust
async fn deploy_events(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    events_sse(&st, id, &headers, sse_event)
}
```

- [ ] **Step 4: Run the suite — the eight stream tests must be untouched**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep "test result"
PATH=$HOME/.cargo/bin:$PATH cargo fmt && PATH=$HOME/.cargo/bin:$PATH make check
```

Expected: cli 17, core 182, daemon 38, all passing, with **no edit to any existing test**. If a test
needed changing, stop and report which and why before committing.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "refactor(daemon): the stream engine is renderer-agnostic"
```

---

### Task 2: The 204 rule

**Files:**
- Modify: `crates/daemon/src/stream.rs`
- Modify: `crates/daemon/src/api.rs` (tests only)

**Interfaces:**
- Consumes: `events_sse` from Task 1
- Produces: no new signature — behaviour only

`EventSource` reconnects whenever the server closes a stream, and our stream closes on the terminal
event. So a finished deploy left open in a browser tab reconnects every few seconds forever. A `204
No Content` response tells `EventSource` not to reconnect; that is the fix, and it belongs in the
engine so both streams get it.

- [ ] **Step 1: Write the failing tests**

In `api.rs`'s test module:

```rust
    /// The reconnect a browser makes after the stream closes. It carries a
    /// `Last-Event-ID` at the end of the log, and there is nothing left to
    /// send — so the answer must be the one that stops `EventSource` from
    /// coming back, not another empty 200 that invites it to.
    #[tokio::test]
    async fn a_reconnect_with_nothing_left_is_a_204_so_the_browser_stops() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);
        let last = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let res = app
            .oneshot(get_resuming(
                &format!("/api/deploys/{id}/events"),
                &last.id.to_string(),
            ))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    /// The *first* connection to a finished deploy is not a reconnect: the
    /// client has seen nothing, so it must still get the whole timeline. A 204
    /// here would leave the page permanently blank.
    #[tokio::test]
    async fn a_first_connection_to_a_finished_deploy_still_gets_its_timeline() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);
        store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}/events")))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(sse_data(res).await.len(), 2);
    }

    /// An in-progress deploy whose events the client has all seen is not
    /// finished — more are coming, so the stream must stay open.
    #[tokio::test]
    async fn an_in_progress_deploy_stays_open_even_when_fully_caught_up() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        let seen = store
            .append_event(&Event::for_stage(id, Stage::Detect, EventStatus::Started, None))
            .expect("append");

        let res = app
            .oneshot(get_resuming(
                &format!("/api/deploys/{id}/events"),
                &seen.id.to_string(),
            ))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);

        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        assert_eq!(sse_data(res).await.len(), 1);
    }
```

- [ ] **Step 2: Run them to verify the first fails**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test -p kuadrat-daemon a_reconnect_with_nothing_left 2>&1 | tail -20
```

Expected: FAIL — `200 OK` where `204` was wanted. The other two pin behaviour that must survive the
change and pass already; say so in the TDD evidence rather than contriving failures.

- [ ] **Step 3: Implement the rule**

In `events_sse`, after `resume` is computed and before the stream is built:

```rust
    let last_id = backlog.last().map_or(0, |ev| ev.id);
    let resume = resume_from(headers).min(last_id);

    // A finished deploy with nothing the client has not already seen. Closing
    // a stream is how this handler says "the deploy ended" — but `EventSource`
    // reads a closed stream as a dropped connection and reconnects a few
    // seconds later, forever. `204 No Content` is the response the HTML
    // specification defines as "do not reconnect", so a finished deploy left
    // open in a tab goes quiet after exactly one extra round trip.
    //
    // Both halves of the condition earn their place. Without `already_terminal`
    // a live deploy that a viewer is caught up with would be told to go away
    // mid-run. Without the seen-everything half, the *first* connection to a
    // finished deploy would get a 204 and render an empty timeline instead of
    // its history.
    if already_terminal && resume >= last_id {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
```

Note this also covers the deploy that ended with no event at all — `reserve` rejecting a duplicate
leaves a terminal row and an empty log, where `last_id` is 0 and "seen everything" is trivially
true.

- [ ] **Step 4: Run the suite**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test --workspace 2>&1 | grep "test result"
PATH=$HOME/.cargo/bin:$PATH cargo fmt && PATH=$HOME/.cargo/bin:$PATH make check
```

Expected: daemon **41** (38 + 3), core 182, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "fix(daemon): a spent stream answers 204 so the browser stops reconnecting"
```

---

### Task 3: The embedded assets

**Files:**
- Create: `crates/daemon/assets/htmx.min.js`, `crates/daemon/assets/sse.min.js`,
  `crates/daemon/assets/kuadrat.css`, `crates/daemon/assets/PROVENANCE.md`
- Create: `crates/daemon/src/assets.rs`
- Modify: `crates/daemon/src/lib.rs`, `crates/daemon/src/api.rs` (router)

**Interfaces:**
- Produces: `pub fn routes() -> Router<AppState>` (or three `get` handlers registered by `router`) —
  serving `/assets/htmx.min.js`, `/assets/sse.min.js`, `/assets/kuadrat.css`

- [ ] **Step 1: Vendor the two upstream files**

```bash
cd /home/kyy/devbox/kuadrat
mkdir -p crates/daemon/assets
curl -sSL -o crates/daemon/assets/htmx.min.js https://unpkg.com/htmx.org@2.0.10/dist/htmx.min.js
curl -sSL -o crates/daemon/assets/sse.min.js  https://unpkg.com/htmx-ext-sse@2.2.4/dist/sse.min.js
sha256sum crates/daemon/assets/htmx.min.js crates/daemon/assets/sse.min.js
```

The hashes must be exactly:

```
71ea67185bfa8c98c39d31717c6fce5d852370fcdfd129db4543774d3145c0de  htmx.min.js
98a46496de0c3605fbffdce9167ba427bdd9553184f83f149c261891a92c0136  sse.min.js
```

**If either hash differs, stop and report it.** These were captured from upstream while writing this
plan; a mismatch means the file changed under a version that is supposed to be immutable, and
vendoring it unexamined is exactly the thing provenance exists to prevent.

- [ ] **Step 2: Record provenance**

Create `crates/daemon/assets/PROVENANCE.md`:

```markdown
# Vendored assets

This is the only third-party code in the repository. It is vendored rather than fetched because the
daemon binds loopback on a host that may have no outbound network — a CDN reference would break the
UI exactly where kuadrat is meant to run.

| File | Upstream | Version | SHA-256 |
|---|---|---|---|
| `htmx.min.js` | https://unpkg.com/htmx.org@2.0.10/dist/htmx.min.js | 2.0.10 | `71ea67185bfa8c98c39d31717c6fce5d852370fcdfd129db4543774d3145c0de` |
| `sse.min.js` | https://unpkg.com/htmx-ext-sse@2.2.4/dist/sse.min.js | 2.2.4 | `98a46496de0c3605fbffdce9167ba427bdd9553184f83f149c261891a92c0136` |

Retrieved 2026-08-11. htmx 2 ships SSE support as a separate extension package, which is why this is
two files. To update: fetch the new version, record its hash here in the same edit, and re-run the
asset tests.

`kuadrat.css` is ours; it has no upstream.
```

- [ ] **Step 3: Write the stylesheet**

Create `crates/daemon/assets/kuadrat.css`. Keep it small and legible — this is an operator tool on
loopback, not a product surface. A system font stack, a readable measure, a table that reads as a
table, and one colour each for the running / stopped / failed states. Do not add a build step, a
preprocessor, or a framework.

- [ ] **Step 4: Write the failing tests**

In a `#[cfg(test)]` module in `crates/daemon/src/assets.rs`:

```rust
#[tokio::test]
async fn htmx_is_served_as_javascript() {
    let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
    let res = app.oneshot(get("/assets/htmx.min.js")).await.expect("send");
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(
        res.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/javascript; charset=utf-8")
    );
}

#[tokio::test]
async fn the_stylesheet_is_served_as_css() {
    let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
    let res = app.oneshot(get("/assets/kuadrat.css")).await.expect("send");
    assert_eq!(
        res.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/css; charset=utf-8")
    );
}

/// A wrong content type is the failure mode here: a browser will not execute a
/// script served as `text/plain`, and the page fails in a way that looks like
/// htmx is broken rather than like the server is.
#[tokio::test]
async fn the_sse_extension_is_served_and_is_not_empty() {
    let (app, _store, _hub, _d) = crate::api::tests::harness_parts();
    let res = app.oneshot(get("/assets/sse.min.js")).await.expect("send");
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1 << 20).await.expect("body");
    assert!(body.len() > 1000, "sse extension looks truncated: {} bytes", body.len());
}
```

Reaching the harness across modules needs `api`'s test helpers visible to `assets`' tests. Make the
`api::tests` module `pub(crate)` and its `harness_parts`/`get` helpers `pub(crate)`; if that fights
the compiler, put these three tests in `api.rs`'s test module instead and say so in the report.
Do not duplicate the harness.

- [ ] **Step 5: Implement**

```rust
//! The embedded UI assets.
//!
//! `include_str!` rather than a runtime read: the binary is the deployment
//! unit, and an asset that can be missing at runtime is a page that breaks on
//! a host nobody can debug from.

const HTMX: &str = include_str!("../assets/htmx.min.js");
const SSE: &str = include_str!("../assets/sse.min.js");
const CSS: &str = include_str!("../assets/kuadrat.css");

const JS: &str = "text/javascript; charset=utf-8";
const CSS_TYPE: &str = "text/css; charset=utf-8";

async fn htmx() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, JS)], HTMX)
}
// ... sse(), css() the same shape
```

Register the three routes in `api::router`.

- [ ] **Step 6: Run the suite**

Expected: daemon **44** (41 + 3), core 182, cli 17. `make check` clean.

- [ ] **Step 7: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): embed htmx, its SSE extension, and the stylesheet"
```

---

### Task 4: `Store::recent_deploys`

**Files:**
- Modify: `crates/core/src/store/mod.rs`

**Interfaces:**
- Produces: `pub fn recent_deploys(&self, app: &str, limit: usize) -> Result<Vec<DeployRow>>` —
  newest first

The one `core` change in this group. `/app/:name` shows an app's history and nothing in `Store`
lists it.

- [ ] **Step 1: Write the failing tests**

```rust
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
        assert!(store.recent_deploys("nothing", 10).expect("read").is_empty());
    }
```

- [ ] **Step 2: Run to verify they fail**

```bash
PATH=$HOME/.cargo/bin:$PATH cargo test -p kuadrat-core recent_deploys 2>&1 | tail -20
```

Expected: compile failure — no method `recent_deploys`.

- [ ] **Step 3: Implement**

```rust
    /// An app's deploy history, newest first, bounded by `limit`.
    ///
    /// Ordered by id rather than by `created_at`: ids are monotonic from
    /// SQLite's `AUTOINCREMENT`, while two deploys created inside the same
    /// second share a timestamp and would order arbitrarily.
    pub fn recent_deploys(&self, app: &str, limit: usize) -> Result<Vec<DeployRow>> {
        let conn = self.conn.lock().expect("store lock");
        let mut stmt = conn
            .prepare(
                "SELECT id, app, stage, status, detail FROM deploys
                 WHERE app = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .context("preparing recent deploys query")?;
        let rows = stmt
            .query_map(params![app, limit as i64], deploy_row)
            .context("querying recent deploys")?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading deploy row")??);
        }
        Ok(out)
    }
```

- [ ] **Step 4: Run the suite**

Expected: core **186** (182 + 4), daemon 44, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src/store/mod.rs
git commit -m "feat(core): list an app's recent deploys"
```

---

### Task 5: The layout, the app list, and the 404 page

**Files:**
- Create: `crates/daemon/src/pages.rs`
- Modify: `crates/daemon/Cargo.toml`, `crates/daemon/src/lib.rs`, `crates/daemon/src/api.rs` (router)

**Interfaces:**
- Consumes: `Store::list_app_configs`, `summarise`/`status`, `AppSummary`
- Produces:
  - `fn layout(title: &str, body: Markup) -> Markup`
  - `async fn index(State<AppState>) -> Markup`
  - `fn not_found(what: &str) -> Response`
  - route `GET /`

- [ ] **Step 1: Add the dependency**

`crates/daemon/Cargo.toml`: `maud = { version = "0.26", features = ["axum"] }` — see the Global
Constraints note on why not 0.27.

- [ ] **Step 2: Write the failing tests**

```rust
    #[tokio::test]
    async fn the_index_lists_a_registered_app_with_its_status() {
        let (app, store, _hub, _d) = harness_parts();
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");

        let res = app.oneshot(get("/")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_text(res).await;
        assert!(body.contains("web"), "app name missing: {body}");
        assert!(body.contains("/srv/web"), "repo path missing");
    }

    #[tokio::test]
    async fn the_index_says_so_when_nothing_is_registered() {
        let (app, _store, _hub, _d) = harness_parts();
        let body = body_text(app.oneshot(get("/")).await.expect("send")).await;
        assert!(
            body.to_lowercase().contains("no apps"),
            "an empty list must say it is empty, not render a bare table: {body}"
        );
    }

    /// The least trusted data in the system reaches these pages: app names come
    /// from an operator, but log lines come from whatever the deployed
    /// application wrote. If anything here ever renders raw, an app that logs
    /// markup rewrites the operator's console. `maud` escapes by default; this
    /// pins that nothing later opts out.
    #[tokio::test]
    async fn interpolated_values_are_escaped_not_rendered() {
        let (app, store, _hub, _d) = harness_parts();
        store
            .register_app(&AppConfig {
                name: "<script>alert(1)</script>".into(),
                repo_path: "/srv/x".into(),
                route: None,
            })
            .expect("register");

        let body = body_text(app.oneshot(get("/")).await.expect("send")).await;
        assert!(
            !body.contains("<script>alert(1)</script>"),
            "raw markup reached the page: {body}"
        );
        assert!(body.contains("&lt;script&gt;"), "expected escaped form: {body}");
    }

    #[tokio::test]
    async fn an_unknown_page_route_answers_html_not_json() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app.oneshot(get("/app/nope")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert!(res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/html"));
    }
```

`body_text` is a new test helper beside `body_json`:

```rust
    async fn body_text(res: Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }
```

The 404 test needs `GET /app/:name` to exist as a route; register it in this task returning
`not_found` for every name, and give it its real body in Task 6. Say so in the report — a route that
only 404s is a stub, and a reviewer should know it is deliberate.

- [ ] **Step 3: Implement**

```rust
//! The operator's pages.
//!
//! Rendered with `maud`, which escapes every interpolation by default. That
//! default is the whole reason it is here: these pages carry journald content,
//! which kuadrat cannot vouch for — `known-gaps.md` records it as "whatever
//! the application wrote to its stdout and stderr". `maud::PreEscaped` does
//! not appear in this module, and should not.

use maud::{html, Markup, DOCTYPE};

/// The shell every page shares.
fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "kuadrat — " (title) }
                link rel="stylesheet" href="/assets/kuadrat.css";
                script src="/assets/htmx.min.js" {}
                script src="/assets/sse.min.js" {}
            }
            body {
                header { a href="/" { "kuadrat" } }
                main { (body) }
            }
        }
    }
}

async fn index(State(st): State<AppState>) -> Markup {
    let configs = st.store.list_app_configs().unwrap_or_default();
    // ... summarise each, then render a table, or a "No apps registered yet."
    // paragraph when the list is empty.
}
```

A store read that fails renders an empty list rather than a 500: the index is where an operator goes
when something is wrong, and it failing closed is worse than it rendering thin. Say so in a comment.

- [ ] **Step 4: Run the suite**

Expected: daemon **48** (44 + 4), core 186, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): the app list, the layout, and an HTML 404"
```

---

### Task 6: The app detail page

**Files:**
- Modify: `crates/daemon/src/pages.rs`

**Interfaces:**
- Consumes: `Store::app_config`, `Store::current_spec`, `Store::recent_deploys` (Task 4),
  `logs::tail`, `workloads::query::status`
- Produces: the real `GET /app/:name` body

Shows: status, route, image, the **10** most recent deploys, and a **100**-line log tail. Both
numbers are fixed by the design document; declare them as named constants, not literals in the
middle of a render function.

- [ ] **Step 1: Write the failing tests**

```rust
    #[tokio::test]
    async fn the_detail_page_shows_the_route_and_the_recent_deploys() {
        let (app, store, _hub, _d) = harness_parts();
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: Some(Route { domain: "example.com".into(), port: 3000 }),
            })
            .expect("register");
        let id = store.create_deploy("web").expect("deploy");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let body = body_text(app.oneshot(get("/app/web")).await.expect("send")).await;
        assert!(body.contains("example.com"), "route missing: {body}");
        assert!(body.contains(&format!("/deploy/{id}")), "no link to the deploy");
    }

    /// A log line is the least trusted string on the page. It must arrive as
    /// text.
    #[tokio::test]
    async fn a_log_line_containing_markup_is_escaped() {
        let (app, store, _hub, _d) = harness_with_log_line("<script>alert(1)</script>");
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");

        let body = body_text(app.oneshot(get("/app/web")).await.expect("send")).await;
        assert!(!body.contains("<script>alert(1)</script>"), "raw log markup rendered");
        assert!(body.contains("&lt;script&gt;"), "expected the escaped form");
    }

    /// One unreadable journal must not blank the page an operator opened to
    /// find out what is wrong.
    #[tokio::test]
    async fn a_failed_log_read_leaves_the_rest_of_the_page_intact() {
        let (app, store, _hub, _d) = harness_with_failing_logs();
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");

        let res = app.oneshot(get("/app/web")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_text(res).await;
        assert!(body.contains("/srv/web"), "the rest of the page is gone: {body}");
        assert!(
            body.to_lowercase().contains("could not read"),
            "the log section must say why it is empty"
        );
    }

    #[tokio::test]
    async fn an_app_with_no_deploys_renders_without_a_history_table() {
        let (app, store, _hub, _d) = harness_parts();
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");

        let res = app.oneshot(get("/app/web")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
    }
```

`harness_with_log_line` and `harness_with_failing_logs` are variants of `harness_with_capacity` whose
`FakeExecutor` scripts `journalctl` — with a line of output, and with a non-zero exit. Follow how
`logs_returns_the_units_lines` in `api.rs` already scripts it; reuse that mechanism rather than
inventing a second one.

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

```rust
/// Deploys shown on an app's page. Fixed by the design document rather than
/// left to taste, so the page and anyone reading the spec agree.
const RECENT_DEPLOYS: usize = 10;

/// Log lines tailed on an app's page — the same default the JSON logs endpoint
/// uses, so the page and the API mean the same thing by "the recent log".
const LOG_LINES: usize = 100;
```

The log section renders one of three things: the lines, a "no output yet" note, or a "could not read
the journal" note carrying the reason. The rest of the page renders regardless.

- [ ] **Step 4: Run the suite**

Expected: daemon **52** (48 + 4), core 186, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): the app detail page"
```

---

### Task 7: The deploy page and its HTML stream

**Files:**
- Modify: `crates/daemon/src/pages.rs`, `crates/daemon/src/api.rs` (router)

**Interfaces:**
- Consumes: `events_sse` (Task 1), `Store::deploy`, `Store::events_for`
- Produces:
  - `fn event_row(ev: &StoredEvent) -> Markup` — used by **both** the server-rendered backlog and the
    stream renderer
  - `GET /deploy/:id`, `GET /deploy/:id/stream`

The one place where the two halves of this group meet. **`event_row` must be the single renderer**:
if the page built its rows one way and the stream another, a live row and a reloaded row would look
different, which is exactly the bug a reader would blame on the stream.

- [ ] **Step 1: Write the failing tests**

```rust
    /// A terminal deploy renders its whole timeline and attaches no stream —
    /// there is nothing to wait for, and an SSE connection that can only close
    /// is a reconnect loop waiting to happen.
    #[tokio::test]
    async fn a_finished_deploy_renders_its_timeline_without_a_stream() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Build, EventStatus::Started);
        store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let body = body_text(app.oneshot(get(&format!("/deploy/{id}"))).await.expect("send")).await;
        assert!(body.contains("build"), "the stored timeline is missing: {body}");
        assert!(!body.contains("sse-connect"), "a finished deploy must not open a stream");
    }

    #[tokio::test]
    async fn an_in_progress_deploy_attaches_to_its_stream() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);

        let body = body_text(app.oneshot(get(&format!("/deploy/{id}"))).await.expect("send")).await;
        assert!(
            body.contains(&format!("sse-connect=\"/deploy/{id}/stream\"")),
            "no stream attached: {body}"
        );
        assert!(body.contains("hx-swap=\"beforeend\""), "rows must append, not replace");
    }

    /// The stream sends the same fragment the page renders. If these diverge, a
    /// row that arrived live looks different from the same row after a reload.
    #[tokio::test]
    async fn the_stream_sends_the_same_row_markup_the_page_renders() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");

        let res = app
            .clone()
            .oneshot(get(&format!("/deploy/{id}/stream")))
            .await
            .expect("send");

        let live = store
            .append_event(&Event::for_stage(id, Stage::Build, EventStatus::Started, None))
            .expect("append");
        hub.emit(&live);
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let streamed = sse_raw_data(res).await;
        assert!(streamed[0].contains("build"), "fragment: {}", streamed[0]);
        assert!(streamed[0].starts_with("<li"), "fragment must be a row: {}", streamed[0]);

        let page = body_text(app.oneshot(get(&format!("/deploy/{id}"))).await.expect("send")).await;
        assert!(
            page.contains(streamed[0].trim()),
            "the page and the stream disagree about a row's markup"
        );
    }

    #[tokio::test]
    async fn an_unknown_deploy_page_is_an_html_404() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app.oneshot(get("/deploy/999")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
```

`sse_raw_data` is `sse_data` without the JSON parse — it returns the `data:` payloads as strings.
Extract the shared part rather than copying it.

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

One rendering function serves both cases, using maud's optional attributes so the live-only
attributes vanish for a terminal deploy:

```rust
fn deploy_page(row: &DeployRow, events: &[StoredEvent], live: bool) -> Markup {
    let connect = live.then(|| format!("/deploy/{}/stream", row.id));
    html! {
        h1 { "Deploy " (row.id) " — " (row.app) }
        p { "Status: " (row.status.as_str()) }
        ul #timeline
            hx-ext=[live.then_some("sse")]
            sse-connect=[connect.as_deref()]
            sse-swap=[live.then_some("message")]
            hx-swap=[live.then_some("beforeend")]
        {
            @for ev in events { (event_row(ev)) }
        }
    }
}
```

and the stream handler renders through the same `event_row`:

```rust
async fn deploy_stream(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    events_sse(&st, id, &headers, |ev| {
        sse::Event::default()
            .id(ev.id.to_string())
            .data(event_row(ev).into_string())
    })
}
```

`maud` renders without inter-element whitespace, so a row is a single line and needs no special
handling as SSE `data:`. If a fragment ever does span lines, axum emits one `data:` per line and the
client rejoins them — correct, but note it if you see it.

- [ ] **Step 4: Run the suite**

Expected: daemon **56** (52 + 4), core 186, cli 17.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): the deploy page and its HTML event stream"
```

---

### Task 8: The actions

**Files:**
- Modify: `crates/daemon/src/api.rs` (content negotiation, the form route), `crates/daemon/src/pages.rs`

**Interfaces:**
- Produces:
  - content negotiation on `POST /api/apps/:name/deploy`
  - `POST /apps` — form-encoded registration
  - a redeploy button on `/app/:name`, a registration form on `/`

- [ ] **Step 1: Write the failing tests**

```rust
    /// A browser posting the redeploy form must land on the page that shows the
    /// deploy it just started.
    #[tokio::test]
    async fn a_browser_deploy_redirects_to_the_deploy_page() {
        let (app, store, _hub, _d) = harness_with_spec();
        // ... register "web" with a repo containing a kuadrat.json

        let res = app
            .oneshot(post_html("/api/apps/web/deploy"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        let location = res.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("");
        assert!(location.starts_with("/deploy/"), "location was {location:?}");
    }

    /// The CLI is a JSON client and must be unaffected. Note this request sends
    /// no `Accept` header at all — the default has to be JSON, or every
    /// existing API caller silently becomes a redirect follower.
    #[tokio::test]
    async fn a_request_without_an_accept_header_still_gets_json() {
        let (app, store, _hub, _d) = harness_with_spec();
        // ... same registration

        let res = app.oneshot(post("/api/apps/web/deploy")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        assert!(body_json(res).await.get("deploy_id").is_some());
    }

    #[tokio::test]
    async fn the_registration_form_registers_and_redirects() {
        let (app, store, _hub, _d) = harness_parts();
        let res = app
            .oneshot(post_form("/apps", "name=web&repo_path=/srv/web"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            res.headers().get("location").and_then(|v| v.to_str().ok()),
            Some("/app/web")
        );
        assert!(store.app_config("web").expect("read").is_some());
    }

    /// A rejected registration must explain itself on the page the operator is
    /// looking at, not as a bare status code.
    #[tokio::test]
    async fn a_rejected_registration_re_renders_the_form_with_the_reason() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app
            .oneshot(post_form("/apps", "name=web&repo_path=relative/path"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = body_text(res).await;
        assert!(body.contains("<form"), "the form must come back: {body}");
        assert!(
            body.to_lowercase().contains("absolute"),
            "the reason must be on the page: {body}"
        );
    }
```

`post_html` sets `Accept: text/html`; `post_form` sets
`content-type: application/x-www-form-urlencoded` with the given body.

`harness_with_spec` is `harness_parts` plus a real repo on disk, because the deploy handler reads
`kuadrat.json` through `std::fs`, not through the `FileSystem` seam:

```rust
    /// The deploy route needs a spec it can actually read: `spec_for` goes
    /// through `std::fs`, so a fake filesystem does not reach it. The repo
    /// lives in the same `TempDir` the store does, so it dies with the test.
    fn harness_with_spec() -> (Router, Arc<Store>, Arc<BroadcastSink>, TempDir) {
        let (app, store, hub, dir) = harness_parts();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo dir");
        let mut spec = WorkloadSpec::new("web", "placeholder");
        spec.ports = vec!["3000:3000".into()];
        std::fs::write(
            repo.join("kuadrat.json"),
            serde_json::to_string(&spec).expect("spec json"),
        )
        .expect("write spec");
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: repo.to_string_lossy().into_owned(),
                route: None,
            })
            .expect("register");
        (app, store, hub, dir)
    }
```

No route, so `validate()` does not demand a `health_cmd`. The deploy this starts will fail in its
background task once it reaches a command the `FakeExecutor` has not scripted — that is fine and
expected. These two tests assert on the *response*, which is returned as soon as the deploy is
reserved, and a failing background deploy cannot change it.

- [ ] **Step 2: Run to verify they fail**

- [ ] **Step 3: Implement**

```rust
/// Whether this caller is a browser.
///
/// The redirect is the exception, not the default: browsers reliably send
/// `text/html` in `Accept`, while an API client that forgets the header would
/// be turned into a redirect follower by the opposite test. Defaulting to JSON
/// keeps every existing caller — the CLI, curl, the tests — working unchanged.
fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html"))
}
```

The deploy handler keeps every existing check and branches only on the way out. The form
registration handler calls the same `register_app` path the JSON route does — one validation
implementation, two presentations.

- [ ] **Step 4: Run the suite**

Expected: daemon **60** (56 + 4), core 186, cli 17. `make check` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): deploy and register from the browser"
```

---

### Task 9: Record what changed

**Files:**
- Modify: `docs/design/2026-08-11-phase-3-daemon-and-surfaces.md`, `docs/known-gaps.md`

- [ ] **Step 1: Amend the parent design's task table**

The H6 row currently reads "The three htmx pages and embedded assets". Replace with:

```markdown
| **H6** | The three htmx pages, embedded assets, the page-facing HTML event stream, and the browser half of deploy and registration |
```

- [ ] **Step 2: Record the 204 rule where the design describes the stream**

In §"The SSE stream", after "The stream closes when the deploy reaches a terminal status.", add:

```markdown
A closed stream is a dropped connection as far as `EventSource` is concerned, and it reconnects. So
a stream with nothing left to send — a terminal deploy whose events the client has all seen —
answers `204 No Content`, which the HTML specification defines as "do not reconnect". Without it a
finished deploy left open in a tab polls the daemon forever.
```

- [ ] **Step 3: Record the vendored assets in known-gaps**

```markdown
## From H6 — vendored frontend assets

`crates/daemon/assets/` carries htmx 2.0.10 and `htmx-ext-sse` 2.2.4, vendored because the daemon
binds loopback on a host that may have no outbound network. They are the only third-party code in
the repository and nothing updates them automatically: a published security advisory against either
will not surface here. `assets/PROVENANCE.md` records the upstream URL, version, and SHA-256 of
each, which is what makes an update auditable — check it when either project publishes a release.
```

- [ ] **Step 4: Record the browser-facing write surface in known-gaps**

Added after the task review of Task 8 raised it. Write it as its own section:

```markdown
## From H6 — the form routes have no CSRF defence

`POST /apps` and the browser branch of `POST /api/apps/:name/deploy` are plain HTML forms: no CSRF
token, no `Origin` or `SameSite` check. They are the first state-changing routes this daemon exposes
to a browser.

Loopback-only is not by itself a defence here. Any page open in the operator's browser can submit a
cross-origin form POST to `127.0.0.1` — the browser will send it, and nothing on these routes
distinguishes it from a click on kuadrat's own page. What loopback does buy is that the attacker
must already have the operator loading their page; what it does not buy is immunity.

Deliberately not fixed in H6: the phase binds loopback and ships no authentication at all, so a
token would be the only control on a surface that has no others, and it would suggest a boundary
that is not there. It must be fixed in the same change that gives the daemon authentication or
reachability beyond loopback — whichever comes first — and not after.
```

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs: record H6's surfaces, the 204 rule, the vendored assets, and the CSRF gap"
```

---

## H6 completion checklist

- [ ] `cargo test --workspace` passes: core 186, daemon 60, cli 17
- [ ] `make check` clean
- [ ] The eight pre-existing JSON stream tests were never edited
- [ ] `maud::PreEscaped` appears nowhere in the repository
- [ ] A log line containing `<script>` renders as text, proven by a test
- [ ] One `event_row` serves both the page and the stream, proven by a test comparing them
- [ ] A finished deploy's reconnect gets `204`; its first connection still gets the timeline
- [ ] Vendored asset hashes match `PROVENANCE.md`
- [ ] A request with no `Accept` header still gets JSON from the deploy route

## Not in H6 (later groups)

- **Live log tailing.** Phase 4. `logs::tail` stays a bounded read.
- **Authentication.** Loopback-only in this phase — log content is as sensitive as the least careful
  app on the host, which is why it binds where it does.
- **Pagination.** `RECENT_DEPLOYS` is a fixed ceiling.
- **A self-refreshing app list.** Deliberately excluded; the design document says why.
- **The webhook sender, `kuadrat serve`, the systemd unit, and `serve-acceptance.sh`.** H7.
