use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/proxy/status", get(proxy_status))
        .route("/proxy/routes", get(list_routes))
        .route("/proxy/ensure", post(ensure_traefik))
}

async fn proxy_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let running = state
        .traefik
        .is_running()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({
        "traefik_running": running,
        "network": crate::services::traefik::TRAEFIK_NETWORK,
        "container": crate::services::traefik::TRAEFIK_CONTAINER,
    })))
}

async fn list_routes(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let routes = state.traefik.list_routes().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;

    Ok(Json(json!(routes)))
}

async fn ensure_traefik(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .traefik
        .ensure_traefik(state.config.traefik_http_port)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({"message": "Traefik is running"})))
}
