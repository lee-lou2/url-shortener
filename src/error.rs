//! Uniform JSON error responses (`{"error": "..."}`) and custom JSON extractor.
//!
//! 4xx errors return safe client-facing messages; 5xx errors log internally,
//! report to Sentry, and emit a generic message without leaking DB or runtime internals.

use axum::{
    extract::{FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{1}")]
    Request(StatusCode, String),
    #[error("Database error")]
    Database(#[from] sqlx::Error),
    #[error("Template error")]
    Template(#[from] askama::Error),
    #[error("{0}")]
    Internal(&'static str),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::Request(StatusCode::BAD_REQUEST, message.into())
    }

    pub fn not_found() -> Self {
        Self::Request(StatusCode::NOT_FOUND, "URL not found".into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Request(status, message) => json_error(status, &message),
            error => {
                tracing::error!(%error, "Request failed");
                sentry::capture_error(&error);
                json_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }
}

pub fn json_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

pub struct ApiJson<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for ApiJson<T> {
    type Rejection = AppError;

    async fn from_request(request: Request, state: &S) -> AppResult<Self> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                let status = error.status();
                let message = match status {
                    StatusCode::PAYLOAD_TOO_LARGE => "Request body too large",
                    StatusCode::UNPROCESSABLE_ENTITY => "Invalid JSON fields",
                    StatusCode::UNSUPPORTED_MEDIA_TYPE => "Expected application/json",
                    _ => "Invalid JSON request",
                };
                AppError::Request(status, message.into())
            })
    }
}
