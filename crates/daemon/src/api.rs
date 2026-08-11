//! The JSON API. Every handler is a thin shell over `core`; nothing here
//! decides anything the CLI would decide differently.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kuadrat_core::deploy::{reserve, run_reserved, Ctx};
use kuadrat_core::events::StoredEvent;
use kuadrat_core::logs::tail;
use kuadrat_core::spec::Route;
use kuadrat_core::store::AppConfig;
use kuadrat_core::workloads::query::status;
use serde::{Deserialize, Serialize};

use crate::state::{spec_for, AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/apps", get(list_apps).post(register))
        .route("/api/apps/:name", get(get_app))
        .route("/api/apps/:name/deploy", post(deploy))
        .route("/api/apps/:name/logs", get(logs))
        .route("/api/deploys/:id", get(get_deploy))
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
        Self {
            id: e.id,
            at: e.at,
            stage: e.event.stage.as_str().to_string(),
            status: e.event.status.as_str().to_string(),
            detail: e.event.detail,
        }
    }
}

// -------------------------------------------------------------------- errors

/// An error with the status the design assigns it. Constructed at the point the
/// condition is detected, so the mapping lives with the check rather than in a
/// catch-all `From` that has to guess.
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl std::fmt::Display) -> Self {
        Self(code, msg.to_string())
    }
    fn not_found(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    fn bad_request(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    fn conflict(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    fn internal(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

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
async fn summarise(st: &AppState, c: AppConfig) -> AppSummary {
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
            sink: &*self.sink,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use kuadrat_core::events::fake::FakeSink;
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
    fn harness() -> (Router, Arc<Store>, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::open(&dir.path().join("k.db")).expect("store"));
        let exec = FakeExecutor::new();
        exec.expect("systemctl", ok());
        let state = AppState::new(
            Arc::new(exec),
            Arc::new(FakeFileSystem::new()),
            Arc::new(FakeSink::new()),
            store.clone(),
            Paths::rooted(dir.path()),
        );
        (router(state), store, dir)
    }

    fn get(path: &str) -> Request<Body> {
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

    async fn body_json(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
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

        let res = app.clone().oneshot(get("/api/apps/ghost")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        let res = app.oneshot(post("/api/apps/ghost/deploy")).await.expect("send");
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

        let res = app.oneshot(post("/api/apps/web/deploy")).await.expect("send");
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

        let res = app.oneshot(post("/api/apps/web/deploy")).await.expect("send");
        assert_eq!(res.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn logs_for_an_unregistered_app_are_a_404_not_an_empty_read() {
        // An empty list would read as "this app is quiet" for an app that does
        // not exist at all.
        let (app, _store, _d) = harness();
        let res = app.oneshot(get("/api/apps/ghost/logs")).await.expect("send");
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
            Arc::new(FakeSink::new()),
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
        assert!(body["lines"][0].as_str().unwrap().contains("up"), "was: {body}");
    }

    #[tokio::test]
    async fn an_unknown_deploy_id_is_a_404() {
        let (app, _store, _d) = harness();
        let res = app.oneshot(get("/api/deploys/999")).await.expect("send");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_deploy_is_readable_with_its_events() {
        use kuadrat_core::deploy::Stage;
        use kuadrat_core::events::{Event, EventStatus};

        let (app, store, _d) = harness();
        let id = store.create_deploy("web").expect("create");
        store
            .append_event(&Event {
                deploy_id: id,
                stage: Stage::Build,
                status: EventStatus::Started,
                detail: None,
            })
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
}
