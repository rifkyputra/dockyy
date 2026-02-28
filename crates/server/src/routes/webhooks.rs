use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/webhooks/github", post(github_webhook))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GitHubPushEvent {
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    after: Option<String>,
    repository: Option<GitHubRepo>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GitHubRepo {
    full_name: Option<String>,
    clone_url: Option<String>,
    ssh_url: Option<String>,
}

async fn github_webhook(
    State(state): State<Arc<AppState>>,
    Json(body): Json<GitHubPushEvent>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let commit_sha = body.after.unwrap_or_default();
    let repo_name = body
        .repository
        .as_ref()
        .and_then(|r| r.full_name.clone())
        .unwrap_or_default();
    let clone_url = body
        .repository
        .as_ref()
        .and_then(|r| r.clone_url.clone())
        .unwrap_or_default();

    tracing::info!(
        "Received webhook for {} commit {}",
        repo_name,
        &commit_sha[..7.min(commit_sha.len())]
    );

    // Find matching repository
    let repo_id = state
        .db
        .with_conn(|conn| {
            let id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM repositories WHERE url = ?1 OR url = ?2 LIMIT 1",
                    rusqlite::params![clone_url, repo_name],
                    |row| row.get(0),
                )
                .ok();
            Ok(id)
        })
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    let repo_id = match repo_id {
        Some(id) => id,
        None => {
            tracing::warn!("No matching repository found for webhook: {}", repo_name);
            return Ok((
                StatusCode::ACCEPTED,
                Json(json!({"message": "No matching repository found, ignoring"})),
            ));
        }
    };

    // Create deployment job
    let job_id = state
        .db
        .with_conn(|conn| {
            let payload = json!({
                "repo_id": repo_id,
                "commit_sha": commit_sha,
                "clone_url": clone_url,
            })
            .to_string();

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

    tracing::info!("Created deploy job {} for repo {}", job_id, repo_id);

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "message": "Deployment queued",
            "job_id": job_id,
            "repo_id": repo_id
        })),
    ))
}
