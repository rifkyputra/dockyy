use axum::{extract::State, routing::get, Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use sysinfo::System;

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

    let sys = System::new();
    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());
    let os_name = System::name().unwrap_or_else(|| "unknown".into());
    let os_version = System::os_version().unwrap_or_else(|| "unknown".into());
    let arch = std::env::consts::ARCH;
    let cpu_cores = sys.physical_core_count().unwrap_or(0);
    let uptime_secs = System::uptime();

    Json(json!({
        "status": "ok",
        "message": "Dockyy API is running",
        "docker": if docker_ok { "connected" } else { "disconnected" },
        "version": env!("CARGO_PKG_VERSION"),
        "hostname": hostname,
        "os": format!("{} {}", os_name, os_version),
        "arch": arch,
        "cpu_cores": cpu_cores,
        "uptime_secs": uptime_secs,
    }))
}

async fn server_metrics(State(state): State<Arc<AppState>>) -> Json<SystemMetrics> {
    let metrics = state.metrics.read().await.clone();
    Json(metrics)
}
