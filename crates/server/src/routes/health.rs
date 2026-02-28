use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::services::monitor::SystemMetrics;
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/health", get(health_check))
}

/// Protected route: `GET /api/metrics`
pub fn metrics_routes() -> Router<Arc<AppState>> {
    Router::new().route("/metrics", get(server_metrics))
}

async fn health_check(State(state): State<Arc<AppState>>) -> Json<Value> {
    let docker_ok = state.docker.list_containers(false).await.is_ok();
    Json(json!({
        "status": "ok",
        "message": "Dockyy API is running",
        "docker": if docker_ok { "connected" } else { "disconnected" },
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn server_metrics(State(state): State<Arc<AppState>>) -> Json<SystemMetrics> {
    let metrics = state.metrics.read().await.clone();
    Json(metrics)
}
