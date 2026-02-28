use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/containers", get(list_containers))
        .route("/containers/{id}/start", post(start_container))
        .route("/containers/{id}/stop", post(stop_container))
        .route("/containers/{id}/restart", post(restart_container))
        .route("/containers/{id}", delete(remove_container))
        .route("/containers/{id}/logs", get(get_logs))
}

#[derive(Deserialize)]
struct ListQuery {
    all: Option<bool>,
}

async fn list_containers(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let all = query.all.unwrap_or(true);
    let containers = state
        .docker
        .list_containers(all)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!(containers)))
}

async fn start_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.docker.start_container(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;
    Ok(Json(json!({"status": "started", "container_id": id})))
}

async fn stop_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.docker.stop_container(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;
    Ok(Json(json!({"status": "stopped", "container_id": id})))
}

async fn restart_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state.docker.restart_container(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
    })?;
    Ok(Json(json!({"status": "restarted", "container_id": id})))
}

async fn remove_container(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    state
        .docker
        .remove_container(&id, true)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;
    Ok(Json(json!({"status": "removed", "container_id": id})))
}

#[derive(Deserialize)]
struct LogsQuery {
    tail: Option<usize>,
}

async fn get_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let tail = query.tail.unwrap_or(100);
    let logs = state
        .docker
        .get_container_logs(&id, tail)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(json!({"logs": logs})))
}
