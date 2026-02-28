use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::db::models::Deployment;
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/deployments", get(list_deployments))
        .route("/deployments/repo/{repo_id}", get(list_by_repo))
        .route("/deployments/{id}", get(get_deployment))
        .route("/deployments/{id}/redeploy", post(redeploy))
}

async fn list_deployments(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Deployment>>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, repo_id, status, commit_sha, image_name, container_id,
                        domain, port, build_log, created_at, updated_at
                 FROM deployments ORDER BY created_at DESC LIMIT 50",
            )?;

            let deployments = stmt
                .query_map([], |row| {
                    Ok(Deployment {
                        id: row.get(0)?,
                        repo_id: row.get(1)?,
                        status: row.get(2)?,
                        commit_sha: row.get(3)?,
                        image_name: row.get(4)?,
                        container_id: row.get(5)?,
                        domain: row.get(6)?,
                        port: row.get(7)?,
                        build_log: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(deployments)
        })
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn list_by_repo(
    State(state): State<Arc<AppState>>,
    Path(repo_id): Path<i64>,
) -> Result<Json<Vec<Deployment>>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, repo_id, status, commit_sha, image_name, container_id,
                        domain, port, build_log, created_at, updated_at
                 FROM deployments WHERE repo_id = ?1 ORDER BY created_at DESC LIMIT 20",
            )?;

            let deployments = stmt
                .query_map([repo_id], |row| {
                    Ok(Deployment {
                        id: row.get(0)?,
                        repo_id: row.get(1)?,
                        status: row.get(2)?,
                        commit_sha: row.get(3)?,
                        image_name: row.get(4)?,
                        container_id: row.get(5)?,
                        domain: row.get(6)?,
                        port: row.get(7)?,
                        build_log: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(deployments)
        })
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn get_deployment(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Deployment>, (StatusCode, Json<Value>)> {
    state
        .db
        .with_conn(|conn| {
            let deployment = conn.query_row(
                "SELECT id, repo_id, status, commit_sha, image_name, container_id,
                        domain, port, build_log, created_at, updated_at
                 FROM deployments WHERE id = ?1",
                [id],
                |row| {
                    Ok(Deployment {
                        id: row.get(0)?,
                        repo_id: row.get(1)?,
                        status: row.get(2)?,
                        commit_sha: row.get(3)?,
                        image_name: row.get(4)?,
                        container_id: row.get(5)?,
                        domain: row.get(6)?,
                        port: row.get(7)?,
                        build_log: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )?;
            Ok(deployment)
        })
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": e.to_string()})),
            )
        })
}

async fn redeploy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Get the original deployment's repo_id, then create a new job
    let repo_id = state
        .db
        .with_conn(|conn| {
            let repo_id: i64 = conn.query_row(
                "SELECT repo_id FROM deployments WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            Ok(repo_id)
        })
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    // Insert a new job for this deployment
    let job_id = state
        .db
        .with_conn(|conn| {
            let payload = json!({"repo_id": repo_id}).to_string();
            conn.execute(
                "INSERT INTO jobs (job_type, payload, status) VALUES ('deploy', ?1, 'pending')",
                [&payload],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    Ok(Json(
        json!({"message": "Redeployment queued", "job_id": job_id}),
    ))
}
