//! JSON error type for the REST API. Handlers return `Result<_, ApiError>` and
//! never panic — every failure becomes a structured `{ "error": ... }` response
//! with an appropriate status code.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use yourmemory_core::storage::StorageError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError { status, message: message.into() }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::FORBIDDEN, message)
    }

    /// Used for both "missing" and "out of scope" so resource existence is not leaked.
    pub fn not_found() -> Self {
        ApiError::new(StatusCode::NOT_FOUND, "not found")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        // Storage failures are internal; don't echo SQL detail to the client.
        tracing::error!(error = %e, "storage error");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
    }
}
