use axum::{
    extract::{ConnectInfo, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
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
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<Value>)> {
    let ip = addr.ip().to_string();

    // Check rate limit
    let (attempts, wait_seconds) = state
        .db
        .check_login_rate_limit(&ip)
        .unwrap_or((0, 0));

    if wait_seconds > 0 {
        tracing::warn!(
            ip = %ip,
            attempts = attempts,
            wait_seconds = wait_seconds,
            "Login attempt rate-limited"
        );
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": format!("Too many login attempts. Try again in {} seconds.", wait_seconds)
            })),
        ));
    }

    if body.username != state.config.admin_username {
        let _ = state.db.record_login_attempt(&ip, false);
        tracing::warn!(ip = %ip, username = %body.username, "Failed login attempt: invalid username");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid credentials"})),
        ));
    }

    let valid = bcrypt::verify(&body.password, &state.config.admin_password_hash)
        .unwrap_or(false);

    if !valid {
        let _ = state.db.record_login_attempt(&ip, false);
        tracing::warn!(ip = %ip, username = %body.username, "Failed login attempt: invalid password");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "Invalid credentials"})),
        ));
    }

    let _ = state.db.record_login_attempt(&ip, true);
    tracing::info!(ip = %ip, username = %body.username, "Successful login");

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
