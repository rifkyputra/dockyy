//! The JSON API. Every handler is a thin shell over `core`; nothing here
//! decides anything the CLI would decide differently.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use kuadrat_core::deploy::{reserve, run_reserved, Ctx};
use kuadrat_core::events::StoredEvent;
use kuadrat_core::logs::tail;
use kuadrat_core::spec::Route;
use kuadrat_core::store::AppConfig;
use kuadrat_core::workloads::query::status;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::state::{spec_for, AppState};
use crate::stream::events_sse;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(crate::pages::index))
        // Stub: only ever 404s in this task. Its real body — the app detail
        // page — is Task 6; it exists here so the HTML 404 has a route to hit.
        .route(
            "/app/:name",
            get(|| async { crate::pages::not_found("app") }),
        )
        .route("/api/apps", get(list_apps).post(register))
        .route("/api/apps/:name", get(get_app))
        .route("/api/apps/:name/deploy", post(deploy))
        .route("/api/apps/:name/logs", get(logs))
        .route("/api/deploys/:id", get(get_deploy))
        .route("/api/deploys/:id/events", get(deploy_events))
        .route("/assets/htmx.min.js", get(crate::assets::htmx))
        .route("/assets/sse.min.js", get(crate::assets::sse))
        .route("/assets/kuadrat.css", get(crate::assets::css))
        .with_state(state)
}

// ---------------------------------------------------------------- wire types

#[derive(Serialize)]
pub struct AppSummary {
    pub name: String,
    pub repo_path: String,
    pub route: Option<Route>,
    /// Host truth, read per request: "Running", "Stopped", "Not installed", …
    pub status: String,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    pub repo_path: String,
    #[serde(default)]
    pub route: Option<Route>,
}

#[derive(Deserialize)]
pub struct LogsQuery {
    /// Lines to return. Clamped to 1..=`logs::MAX_LINES` by `core`; the default keeps a
    /// page-sized read from needing a parameter.
    #[serde(default)]
    pub n: Option<usize>,
}

#[derive(Serialize)]
pub struct LogsOut {
    pub name: String,
    pub lines: Vec<String>,
}

#[derive(Serialize)]
pub struct DeployAccepted {
    pub deploy_id: i64,
}

#[derive(Serialize)]
pub struct DeployDetail {
    pub id: i64,
    pub app: String,
    pub stage: String,
    pub status: String,
    pub detail: Option<String>,
    pub events: Vec<EventOut>,
}

#[derive(Serialize)]
pub struct EventOut {
    pub id: i64,
    pub at: String,
    pub stage: String,
    pub status: String,
    pub detail: Option<String>,
}

impl From<StoredEvent> for EventOut {
    fn from(e: StoredEvent) -> Self {
        // The same projection the store writes, so a streamed event, a fetched
        // event, and the database row all spell a stage the same way.
        let (stage, status) = e.event.kind.columns();
        Self {
            id: e.id,
            at: e.at,
            stage: stage.to_string(),
            status: status.to_string(),
            detail: e.event.detail,
        }
    }
}

// ------------------------------------------------------------------ handlers

async fn list_apps(State(st): State<AppState>) -> ApiResult<Json<Vec<AppSummary>>> {
    let configs = st
        .store
        .list_app_configs()
        .map_err(|e| ApiError::internal(format!("reading registrations: {e:#}")))?;

    let mut out = Vec::with_capacity(configs.len());
    for c in configs {
        out.push(summarise(&st, c).await);
    }
    Ok(Json(out))
}

async fn get_app(
    State(st): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<AppSummary>> {
    let config = registration(&st, &name)?;
    Ok(Json(summarise(&st, config).await))
}

async fn register(
    State(st): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<(StatusCode, Json<AppSummary>)> {
    let config = AppConfig {
        name: req.name,
        repo_path: req.repo_path,
        route: req.route,
    };
    // register_app rejects a relative repo_path and a name/slug collision; both
    // are the caller's mistake, so both are 400 rather than 500.
    st.store
        .register_app(&config)
        .map_err(ApiError::bad_request)?;

    Ok((StatusCode::CREATED, Json(summarise(&st, config).await)))
}

/// Trigger a deploy of a registered app. No body — the repo path and route come
/// from the registration, which is the whole point of registering.
///
/// Returns as soon as the deploy is *accepted*. Whether it succeeded arrives
/// over the event stream: this mirrors `deploy::run`'s own contract, where
/// "could not begin" is `Err` and "ran and rolled back" is `Ok(RolledBack)`.
async fn deploy(
    State(st): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<(StatusCode, Json<DeployAccepted>)> {
    let config = registration(&st, &name)?;

    // "This app is already deploying" outranks anything wrong with the spec:
    // while a deploy is in flight the request cannot proceed whatever the spec
    // says, and a 400 telling the operator to fix a kuadrat.json would send
    // them after the wrong problem. `reserve` re-checks atomically below — this
    // read only fixes which error the caller sees, not whether two deploys can
    // start.
    let busy = st
        .store
        .in_progress_deploys()
        .map_err(|e| ApiError::internal(format!("reading in-progress deploys: {e:#}")))?
        .iter()
        .any(|row| row.app == name);
    if busy {
        return Err(ApiError::conflict(format!(
            "another deploy of {name} is already in progress"
        )));
    }

    let repo = std::path::PathBuf::from(&config.repo_path);
    let spec = spec_for(&config, |p| std::fs::read_to_string(p).ok())
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    // Fail the obviously-invalid spec here, where it can be a 400, rather than
    // inside the deploy where it would only be visible as a failed run. A route
    // without a health_cmd is the common one, and it reads as unrelated to the
    // domain field the operator just edited.
    spec.validate()
        .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;

    // Reserve BEFORE waiting for the slot. A duplicate deploy of a busy app is
    // knowable now, so it must be rejected now — queued behind the semaphore it
    // would sit for minutes only to be refused on reaching the front.
    let deploy_id = {
        let ctx = st.ctx();
        reserve(&ctx, &name).map_err(|e| ApiError::conflict(format!("{e:#}")))?
    };

    let bg = st.clone();
    tokio::spawn(async move {
        // One deploy at a time, globally. The permit is held for the whole run
        // and released on drop, including on panic.
        let _permit = match bg.deploy_slot.acquire().await {
            Ok(p) => p,
            // Only if the semaphore was closed — the daemon is shutting down.
            Err(_) => return,
        };
        let ctx = bg.ctx();
        let _ = run_reserved(&ctx, spec, &repo, deploy_id).await;
    });

    Ok((StatusCode::OK, Json(DeployAccepted { deploy_id })))
}

/// A bounded journald read for a registered app. Live tailing is deliberately
/// absent — it lands in phase 4, which has two consumers for it.
async fn logs(
    State(st): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<LogsOut>> {
    // 404 for an app nobody registered, rather than an empty read that looks
    // like a quiet app.
    registration(&st, &name)?;

    let lines = tail(&*st.exec, &name, q.n.unwrap_or(100))
        .await
        .map_err(|e| ApiError::internal(format!("reading logs for {name}: {e:#}")))?;

    Ok(Json(LogsOut { name, lines }))
}

async fn get_deploy(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeployDetail>> {
    let row = st
        .store
        .deploy(id)
        .map_err(|e| ApiError::internal(format!("reading deploy {id}: {e:#}")))?
        .ok_or_else(|| ApiError::not_found(format!("no deploy {id}")))?;

    let events = st
        .store
        .events_for(id)
        .map_err(|e| ApiError::internal(format!("reading events for {id}: {e:#}")))?;

    Ok(Json(DeployDetail {
        id: row.id,
        app: row.app,
        stage: row.stage.as_str().to_string(),
        status: row.status.as_str().to_string(),
        detail: row.detail,
        events: events.into_iter().map(EventOut::from).collect(),
    }))
}

async fn deploy_events(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    events_sse(&st, id, &headers, sse_event)
}

/// One event on the wire.
///
/// The SSE `id` is the store's id, which is what makes `Last-Event-ID`
/// resumption work without the handler keeping any per-connection state. The
/// payload is the same `EventOut` the JSON API returns, so a page renders a
/// streamed event and a fetched one through one code path.
fn sse_event(ev: &StoredEvent) -> sse::Event {
    let out = EventOut::from(ev.clone());
    let id = out.id.to_string();
    sse::Event::default()
        .id(id)
        // `EventOut` is five owned scalars; serialization cannot fail.
        .json_data(&out)
        .expect("EventOut serializes")
}

// ------------------------------------------------------------------- helpers

fn registration(st: &AppState, name: &str) -> ApiResult<AppConfig> {
    st.store
        .app_config(name)
        .map_err(|e| ApiError::internal(format!("reading registration: {e:#}")))?
        .ok_or_else(|| ApiError::not_found(format!("no app {name}")))
}

/// A registration plus the host's current answer for it. A status read that
/// fails is reported as a status, not as a failed request — one unreadable unit
/// must not blank the whole app list.
pub(crate) async fn summarise(st: &AppState, c: AppConfig) -> AppSummary {
    let state = match status(&*st.exec, &*st.fsys, &st.paths, &c.name).await {
        Ok(s) => s.label().to_string(),
        Err(_) => "Unknown".to_string(),
    };
    AppSummary {
        name: c.name,
        repo_path: c.repo_path,
        route: c.route,
        status: state,
    }
}

impl AppState {
    /// Borrow the state as a `core` context. Cheap; built per use because `Ctx`
    /// borrows and cannot be stored alongside what it borrows from.
    pub fn ctx(&self) -> Ctx<'_> {
        Ctx {
            exec: &*self.exec,
            fsys: &*self.fsys,
            store: &self.store,
            paths: &self.paths,
            sink: &*self.hub,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hub::BroadcastSink;
    use axum::body::Body;
    use axum::http::Request;
    use kuadrat_core::deploy::{DeployStatus, Stage};
    use kuadrat_core::events::{Event, EventSink, EventStatus};
    use kuadrat_core::exec::fake::FakeExecutor;
    use kuadrat_core::exec::CommandOutput;
    use kuadrat_core::fs::fake::FakeFileSystem;
    use kuadrat_core::store::Store;
    use kuadrat_core::workloads::paths::Paths;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn ok() -> CommandOutput {
        CommandOutput {
            status: 0,
            stdout: "active".into(),
            stderr: String::new(),
        }
    }

    /// A router over fakes and a temp-file store. No socket is bound and no
    /// podman is required, so these run anywhere the unit tests do.
    pub(crate) fn harness_parts() -> (Router, Arc<Store>, Arc<BroadcastSink>, TempDir) {
        harness_with_capacity(256)
    }

    fn harness_with_capacity(capacity: usize) -> (Router, Arc<Store>, Arc<BroadcastSink>, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open(&dir.path().join("k.db")).expect("store"));
        let exec = FakeExecutor::new();
        exec.expect("systemctl", ok());
        let mut state = AppState::new(
            Arc::new(exec),
            Arc::new(FakeFileSystem::new()),
            store.clone(),
            Paths::rooted(dir.path()),
        );
        state.hub = Arc::new(BroadcastSink::with_capacity(capacity));
        let hub = state.hub.clone();
        (router(state), store, hub, dir)
    }

    fn harness() -> (Router, Arc<Store>, TempDir) {
        let (app, store, _hub, dir) = harness_parts();
        (app, store, dir)
    }

    pub(crate) fn get(path: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("request")
    }

    fn post_json(path: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request")
    }

    fn post(path: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(path)
            .body(Body::empty())
            .expect("request")
    }

    fn get_resuming(path: &str, last: &str) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header("last-event-id", last)
            .body(Body::empty())
            .expect("request")
    }

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    async fn body_text(res: Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[tokio::test]
    async fn the_app_list_is_empty_before_anything_is_registered() {
        let (app, _store, _d) = harness();
        let res = app.oneshot(get("/api/apps")).await.expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(body_json(res).await, serde_json::json!([]));
    }

    #[tokio::test]
    async fn registering_returns_201_and_the_app_appears_in_the_list() {
        let (app, _store, _d) = harness();

        let res = app
            .clone()
            .oneshot(post_json(
                "/api/apps",
                serde_json::json!({"name": "web", "repo_path": "/srv/web"}),
            ))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::CREATED);

        let res = app.oneshot(get("/api/apps")).await.expect("send");
        let body = body_json(res).await;
        assert_eq!(body[0]["name"], "web");
        assert_eq!(body[0]["repo_path"], "/srv/web");
    }

    #[tokio::test]
    async fn a_relative_repo_path_is_rejected_as_a_400() {
        // The daemon runs under systemd with / as its working directory, not
        // the operator's shell, so a relative path resolves against the wrong
        // place. That is the caller's mistake, not a server fault.
        let (app, _store, _d) = harness();
        let res = app
            .oneshot(post_json(
                "/api/apps",
                serde_json::json!({"name": "web", "repo_path": "srv/web"}),
            ))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_unknown_app_is_a_404_on_read_and_on_deploy() {
        let (app, _store, _d) = harness();

        let res = app
            .clone()
            .oneshot(get("/api/apps/ghost"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        let res = app
            .oneshot(post("/api/apps/ghost/deploy"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn deploying_a_registered_app_with_no_spec_is_a_400() {
        // Registered, but the repo has no kuadrat.json and the app has never
        // deployed, so there is nothing to deploy — the operator's problem,
        // reported before anything is reserved.
        let (app, _store, _d) = harness();
        app.clone()
            .oneshot(post_json(
                "/api/apps",
                serde_json::json!({"name": "web", "repo_path": "/nonexistent/web"}),
            ))
            .await
            .expect("send");

        let res = app
            .oneshot(post("/api/apps/web/deploy"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_second_deploy_of_a_busy_app_is_a_409_not_a_queue() {
        // The per-app lock and the global semaphore answer differently: this
        // app being busy is knowable now, so it is refused now rather than
        // queued for minutes and refused at the front.
        let (app, store, _d) = harness();
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");
        let id = store.create_deploy("web").expect("create");
        assert!(store.acquire_lock("web", id).expect("lock"));

        let res = app
            .oneshot(post("/api/apps/web/deploy"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn logs_for_an_unregistered_app_are_a_404_not_an_empty_read() {
        // An empty list would read as "this app is quiet" for an app that does
        // not exist at all.
        let (app, _store, _d) = harness();
        let res = app
            .oneshot(get("/api/apps/ghost/logs"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn logs_returns_the_units_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open(&dir.path().join("k.db")).expect("store"));
        store
            .register_app(&AppConfig {
                name: "web".into(),
                repo_path: "/srv/web".into(),
                route: None,
            })
            .expect("register");
        let exec = FakeExecutor::new();
        exec.expect(
            "journalctl",
            CommandOutput {
                status: 0,
                stdout: "2026-08-11T10:00:00+0000 host web[1]: up\n".into(),
                stderr: String::new(),
            },
        );
        let state = AppState::new(
            Arc::new(exec),
            Arc::new(FakeFileSystem::new()),
            store,
            Paths::rooted(dir.path()),
        );

        let res = router(state)
            .oneshot(get("/api/apps/web/logs?n=50"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);

        let body = body_json(res).await;
        assert_eq!(body["name"], "web");
        assert!(
            body["lines"][0].as_str().unwrap().contains("up"),
            "was: {body}"
        );
    }

    #[tokio::test]
    async fn an_unknown_deploy_id_is_a_404() {
        let (app, _store, _d) = harness();
        let res = app.oneshot(get("/api/deploys/999")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_deploy_is_readable_with_its_events() {
        let (app, store, _d) = harness();
        let id = store.create_deploy("web").expect("create");
        store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Started,
                None,
            ))
            .expect("event");

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}")))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);

        let body = body_json(res).await;
        assert_eq!(body["app"], "web");
        assert_eq!(body["events"][0]["stage"], "build");
        assert_eq!(body["events"][0]["status"], "started");
    }

    /// Read an SSE body to completion and return the `data:` payloads. This
    /// only terminates because the stream closes on the terminal event — which
    /// is the property being tested as much as the contents are.
    async fn sse_data(res: Response) -> Vec<serde_json::Value> {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec())
            .expect("utf8")
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|d| serde_json::from_str(d).expect("json"))
            .collect()
    }

    fn stage_event(store: &Store, id: i64, stage: Stage, status: EventStatus) {
        store
            .append_event(&Event::for_stage(id, stage, status, None))
            .expect("append");
    }

    #[tokio::test]
    async fn an_unknown_deploy_is_a_404_rather_than_an_empty_stream() {
        let (app, _store, _hub, _d) = harness_parts();
        let res = app
            .oneshot(get("/api/deploys/99/events"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    /// A deploy that ended before anyone connected: the whole story is in the
    /// backlog, and the stream must close rather than wait for events that can
    /// never come.
    #[tokio::test]
    async fn a_finished_deploy_streams_its_backlog_and_closes() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);
        stage_event(&store, id, Stage::Detect, EventStatus::Succeeded);
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

        let data = sse_data(res).await;
        assert_eq!(data.len(), 3);
        assert_eq!(data[0]["stage"], "detect");
        assert_eq!(data[2]["stage"], "deploy");
        assert_eq!(data[2]["status"], "done");
    }

    /// A deploy terminated by a path that emits no event — `reserve` rejecting
    /// a duplicate — has a terminal row and an empty log. Without the row
    /// check the stream would wait forever on a deploy that can never speak.
    #[tokio::test]
    async fn a_terminal_deploy_with_no_events_closes_immediately() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        store
            .finish_deploy(id, DeployStatus::Failed, Some("rejected"))
            .expect("finish");

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}/events")))
            .await
            .expect("send");
        assert!(sse_data(res).await.is_empty());
    }

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
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
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

    /// Backlog then live, in order and without a gap — the first of the three
    /// cases the design names.
    #[tokio::test]
    async fn the_backlog_is_followed_by_live_events_in_order() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        stage_event(&store, id, Stage::Detect, EventStatus::Started);

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}/events")))
            .await
            .expect("send");

        // Emitted after the handler has read its backlog and subscribed, so
        // these arrive over the channel rather than from SQLite.
        let live = store
            .append_event(&Event::for_stage(
                id,
                Stage::Build,
                EventStatus::Started,
                None,
            ))
            .expect("append");
        hub.emit(&live);
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let data = sse_data(res).await;
        let stages: Vec<&str> = data
            .iter()
            .map(|d| d["stage"].as_str().expect("stage"))
            .collect();
        assert_eq!(stages, ["detect", "build", "deploy"]);
    }

    /// An event delivered both ways at the join is sent once — the second of
    /// the design's three cases. The handler subscribes before reading, so an
    /// event already in the backlog can also arrive live; the id filter is
    /// what makes that harmless.
    #[tokio::test]
    async fn an_event_in_both_the_backlog_and_the_channel_is_sent_once() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        let dup = store
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("append");

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}/events")))
            .await
            .expect("send");

        hub.emit(&dup); // the same event, arriving live
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let data = sse_data(res).await;
        assert_eq!(data.len(), 2, "the duplicate must be dropped: {data:?}");
    }

    /// Two deploys share one hub. A stream must not leak another deploy's
    /// events into this one's timeline.
    #[tokio::test]
    async fn another_deploys_events_are_not_forwarded() {
        let (app, store, hub, _d) = harness_parts();
        let mine = store.create_deploy("web").expect("create");
        let other = store.create_deploy("api").expect("create");

        let res = app
            .oneshot(get(&format!("/api/deploys/{mine}/events")))
            .await
            .expect("send");

        let theirs = store
            .append_event(&Event::for_stage(
                other,
                Stage::Build,
                EventStatus::Started,
                None,
            ))
            .expect("append");
        hub.emit(&theirs);
        let end = store
            .append_event(&Event::finished(mine, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let data = sse_data(res).await;
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["stage"], "deploy");
    }

    /// The third of the design's three cases. A viewer whose connection is
    /// slower than the deploy loses messages from the channel — but not from
    /// SQLite, because events are persisted before they are published. The
    /// stream must re-read and carry on, not close and not skip.
    #[tokio::test]
    async fn a_lagged_subscriber_recovers_every_missed_event_from_the_store() {
        let (app, store, hub, _d) = harness_with_capacity(2);
        let id = store.create_deploy("web").expect("create");

        let res = app
            .oneshot(get(&format!("/api/deploys/{id}/events")))
            .await
            .expect("send");

        // Six events into a two-slot channel, with nobody polling the body
        // yet: the receiver is guaranteed to be told it lagged.
        for (stage, status) in [
            (Stage::Detect, EventStatus::Started),
            (Stage::Detect, EventStatus::Succeeded),
            (Stage::Build, EventStatus::Started),
            (Stage::Build, EventStatus::Succeeded),
            (Stage::Apply, EventStatus::Started),
            (Stage::Apply, EventStatus::Succeeded),
        ] {
            let ev = store
                .append_event(&Event::for_stage(id, stage, status, None))
                .expect("append");
            hub.emit(&ev);
        }
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        let data = sse_data(res).await;
        let ids: Vec<i64> = data.iter().map(|d| d["id"].as_i64().expect("id")).collect();
        assert_eq!(ids.len(), 7, "nothing may be skipped: {data:?}");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ids must ascend: {ids:?}"
        );
        assert_eq!(data[6]["stage"], "deploy");
    }

    #[tokio::test]
    async fn a_reconnecting_client_gets_only_what_it_has_not_seen() {
        let (app, store, _hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");
        let first = store
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("append");
        stage_event(&store, id, Stage::Build, EventStatus::Started);
        store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        store
            .finish_deploy(id, DeployStatus::Done, None)
            .expect("finish");

        let res = app
            .oneshot(get_resuming(
                &format!("/api/deploys/{id}/events"),
                &first.id.to_string(),
            ))
            .await
            .expect("send");

        let data = sse_data(res).await;
        assert_eq!(data.len(), 2, "the already-seen event must not repeat");
        assert_eq!(data[0]["stage"], "build");
    }

    /// A `Last-Event-ID` past anything the store has ever assigned this deploy
    /// is impossible for a real client to have seen. Trusting it verbatim
    /// would filter out every backlog and live event, including the terminal
    /// one, and the stream would never yield anything or close — this is the
    /// hang Finding 3 closes. Clamping it must still let the stream deliver
    /// live events and close on the terminal one.
    #[tokio::test]
    async fn a_last_event_id_past_the_end_does_not_stall_the_stream() {
        let (app, store, hub, _d) = harness_parts();
        let id = store.create_deploy("web").expect("create");

        let res = app
            .oneshot(get_resuming(
                &format!("/api/deploys/{id}/events"),
                &i64::MAX.to_string(),
            ))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);

        let live = store
            .append_event(&Event::for_stage(
                id,
                Stage::Detect,
                EventStatus::Started,
                None,
            ))
            .expect("append");
        hub.emit(&live);
        let end = store
            .append_event(&Event::finished(id, DeployStatus::Done, None))
            .expect("append");
        hub.emit(&end);

        // This await only returns because the stream actually closes; a stall
        // here is the bug, not a slow assertion.
        let data = sse_data(res).await;
        assert_eq!(
            data.len(),
            2,
            "the bogus resume point must not swallow events"
        );
        assert_eq!(data[0]["stage"], "detect");
        assert_eq!(data[1]["stage"], "deploy");
    }

    /// A header that is not a number is a hint, not a command. Failing the
    /// request over it would break a reconnect for no gain; replaying from the
    /// start is always correct, only chattier.
    #[tokio::test]
    async fn a_malformed_last_event_id_is_treated_as_absent() {
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
            .oneshot(get_resuming(&format!("/api/deploys/{id}/events"), "banana"))
            .await
            .expect("send");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(sse_data(res).await.len(), 2);
    }

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
        assert!(
            body.contains("&lt;script&gt;"),
            "expected escaped form: {body}"
        );
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
}
