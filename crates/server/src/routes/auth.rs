use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::auth as jwt;
use crate::db::models::{LoginRequest, LoginResponse};
use crate::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/verify", post(verify))
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<Value>)> {
    if body.username != state.config.admin_username {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid credentials"})),
        ));
    }

    let valid = bcrypt::verify(&body.password, &state.config.admin_password_hash)
        .unwrap_or(false);

    if !valid {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid credentials"})),
        ));
    }

    let token = jwt::create_token(&state.config.jwt_secret, &body.username)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Failed to create token"})),
            )
        })?;

    Ok(Json(LoginResponse {
        token,
        username: body.username,
    }))
}

async fn verify(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let token = body["token"]
        .as_str()
        .ok_or((StatusCode::BAD_REQUEST, Json(json!({"error": "Missing token"}))))?;

    let claims = jwt::verify_token(&state.config.jwt_secret, token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, Json(json!({"error": "Invalid token"}))))?;

    Ok(Json(json!({
        "valid": true,
        "username": claims.sub
    })))
}
