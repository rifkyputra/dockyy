use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// An error with the status the design assigns it. Constructed at the point the
/// condition is detected, so the mapping lives with the check rather than in a
/// catch-all `From` that has to guess.
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl std::fmt::Display) -> Self {
        Self(code, msg.to_string())
    }
    pub fn not_found(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    pub fn bad_request(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    pub fn unauthorized(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, msg)
    }
    pub fn conflict(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    pub fn internal(msg: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
    pub(crate) fn is_conflict(&self) -> bool {
        self.0 == StatusCode::CONFLICT
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
