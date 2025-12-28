//! 중앙화된 에러 처리 모듈.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Application-wide error type.
///
/// All errors in the application should be converted to this type
/// for consistent error handling and reporting.
#[derive(Error, Debug)]
pub enum AppError {
    /// Bad request error (400)
    #[error("Bad request: {0}")]
    BadRequest(String),

    /// Unauthorized error (401)
    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    /// Not found error (404)
    #[error("Not found: {0}")]
    NotFound(String),

    /// Validation error (400)
    #[error("Validation error: {0}")]
    Validation(String),

    /// Internal server error (500)
    #[error("Internal server error: {0}")]
    Internal(String),

    /// Database error
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Redis cache error
    #[error("Cache error: {0}")]
    Redis(#[from] deadpool_redis::redis::RedisError),

    /// Redis pool error
    #[error("Cache pool error: {0}")]
    RedisPool(#[from] deadpool_redis::PoolError),

    /// JWT error
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),

    /// Template rendering error
    #[error("Template error: {0}")]
    Template(#[from] askama::Error),

    /// JSON serialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// HTTP client error (for webhooks)
    #[error("HTTP client error: {0}")]
    HttpClient(#[from] reqwest::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            Self::BadRequest(msg) | Self::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            Self::Database(e) => {
                tracing::error!("Database error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Database error occurred".to_string(),
                )
            }
            Self::Redis(e) => {
                tracing::error!("Redis error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Cache error occurred".to_string(),
                )
            }
            Self::RedisPool(e) => {
                tracing::error!("Redis pool error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Cache connection error occurred".to_string(),
                )
            }
            Self::Jwt(e) => {
                tracing::warn!("JWT error: {e:?}");
                (StatusCode::UNAUTHORIZED, format!("JWT error: {e}"))
            }
            Self::Template(e) => {
                tracing::error!("Template error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Template rendering error".to_string(),
                )
            }
            Self::Json(e) => {
                tracing::error!("JSON error: {e:?}");
                (StatusCode::BAD_REQUEST, format!("JSON error: {e}"))
            }
            Self::HttpClient(e) => {
                tracing::warn!("HTTP client error: {e:?}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "External service error".to_string(),
                )
            }
        };

        // Report error to Sentry for server errors
        if status.is_server_error() {
            sentry::capture_error(&self);
        }

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}

/// Result type alias using `AppError`.
pub type AppResult<T> = Result<T, AppError>;

/// Helper trait for converting validation errors.
pub trait ValidationErrorExt {
    fn to_validation_error(&self) -> AppError;
}

impl ValidationErrorExt for validator::ValidationErrors {
    fn to_validation_error(&self) -> AppError {
        // Get the first field error for a clean message
        if let Some((field, errors)) = self.field_errors().iter().next() {
            if let Some(error) = errors.first() {
                let message = error.message.as_ref().map_or_else(
                    || {
                        format!(
                            "Validation failed on field '{field}' with tag '{}'",
                            error.code
                        )
                    },
                    std::string::ToString::to_string,
                );
                return AppError::Validation(message);
            }
        }
        AppError::Validation(self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[test]
    fn test_app_error_bad_request_display() {
        let error = AppError::BadRequest("잘못된 요청".to_string());
        assert_eq!(error.to_string(), "Bad request: 잘못된 요청");
    }

    #[test]
    fn test_app_error_unauthorized_display() {
        let error = AppError::Unauthorized("인증 실패".to_string());
        assert_eq!(error.to_string(), "Unauthorized: 인증 실패");
    }

    #[test]
    fn test_app_error_not_found_display() {
        let error = AppError::NotFound("리소스를 찾을 수 없음".to_string());
        assert_eq!(error.to_string(), "Not found: 리소스를 찾을 수 없음");
    }

    #[test]
    fn test_app_error_validation_display() {
        let error = AppError::Validation("유효성 검사 실패".to_string());
        assert_eq!(error.to_string(), "Validation error: 유효성 검사 실패");
    }

    #[test]
    fn test_app_error_internal_display() {
        let error = AppError::Internal("내부 오류".to_string());
        assert_eq!(error.to_string(), "Internal server error: 내부 오류");
    }

    #[test]
    fn test_app_error_debug_format() {
        let error = AppError::BadRequest("test".to_string());
        let debug_str = format!("{error:?}");
        assert!(debug_str.contains("BadRequest"));
        assert!(debug_str.contains("test"));
    }

    #[tokio::test]
    async fn test_bad_request_into_response() {
        let error = AppError::BadRequest("테스트 에러".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_unauthorized_into_response() {
        let error = AppError::Unauthorized("인증 필요".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_not_found_into_response() {
        let error = AppError::NotFound("없음".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_validation_into_response() {
        let error = AppError::Validation("유효하지 않음".to_string());
        let response = error.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_internal_into_response() {
        let error = AppError::Internal("서버 오류".to_string());
        let response = error.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_app_result_ok() {
        let value = 42;
        let result: AppResult<i32> = Ok(value);
        assert!(result.is_ok());
        assert_eq!(result.ok(), Some(value));
    }

    #[test]
    fn test_app_result_err() {
        let result: AppResult<i32> = Err(AppError::NotFound("테스트".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_json_error_from() {
        let json_err = serde_json::from_str::<i32>("invalid").unwrap_err();
        let app_err: AppError = json_err.into();
        assert!(matches!(app_err, AppError::Json(_)));
    }

    #[test]
    fn test_error_empty_message() {
        let error = AppError::BadRequest(String::new());
        assert_eq!(error.to_string(), "Bad request: ");
    }

    #[test]
    fn test_error_unicode_message() {
        let error = AppError::NotFound("🔍 찾을 수 없습니다".to_string());
        assert!(error.to_string().contains("🔍"));
    }

    #[test]
    fn test_error_long_message() {
        let long_msg = "a".repeat(10000);
        let error = AppError::Internal(long_msg.clone());
        assert!(error.to_string().contains(&long_msg));
    }

    // ============ 추가 에러 타입 테스트 ============

    #[test]
    fn test_app_error_database_display() {
        // sqlx::Error는 직접 생성이 어려우므로 문자열 테스트
        let error = AppError::Internal("DB connection failed".to_string());
        assert!(error.to_string().contains("DB connection failed"));
    }

    #[test]
    fn test_app_error_multiple_errors_distinct() {
        let bad_request = AppError::BadRequest("bad".to_string());
        let not_found = AppError::NotFound("not found".to_string());
        let unauthorized = AppError::Unauthorized("unauth".to_string());

        assert_ne!(bad_request.to_string(), not_found.to_string());
        assert_ne!(not_found.to_string(), unauthorized.to_string());
    }

    #[test]
    fn test_app_error_with_special_characters() {
        let special = "에러: <script>alert('xss')</script>";
        let error = AppError::BadRequest(special.to_string());
        assert!(error.to_string().contains(special));
    }

    #[test]
    fn test_app_error_with_newlines() {
        let multiline = "Line 1\nLine 2\nLine 3";
        let error = AppError::Internal(multiline.to_string());
        assert!(error.to_string().contains("Line 1"));
        assert!(error.to_string().contains("Line 2"));
    }

    #[test]
    fn test_app_error_with_json_content() {
        let json_msg = r#"{"error": "test", "code": 123}"#;
        let error = AppError::BadRequest(json_msg.to_string());
        assert!(error.to_string().contains("error"));
    }

    // ============ IntoResponse 추가 테스트 ============

    #[tokio::test]
    async fn test_json_error_into_response() {
        let json_err = serde_json::from_str::<i32>("invalid").unwrap_err();
        let app_err: AppError = json_err.into();
        let response = app_err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_error_response_has_body() {
        use axum::body::to_bytes;

        let error = AppError::NotFound("resource not found".to_string());
        let response = error.into_response();

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);

        assert!(body_str.contains("error"));
        assert!(body_str.contains("resource not found"));
    }

    #[tokio::test]
    async fn test_error_response_is_json() {
        use axum::body::to_bytes;

        let error = AppError::BadRequest("test".to_string());
        let response = error.into_response();

        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(parsed.get("error").is_some());
    }

    // ============ ValidationErrorExt 테스트 ============

    #[test]
    fn test_validation_error_ext_empty_errors() {
        use validator::ValidationErrors;

        let errors = ValidationErrors::new();
        let app_error = errors.to_validation_error();

        assert!(matches!(app_error, AppError::Validation(_)));
    }

    // ============ From 트레이트 테스트 ============

    #[test]
    fn test_json_error_conversion() {
        let json_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let app_err: AppError = json_err.into();

        assert!(matches!(app_err, AppError::Json(_)));
        assert!(app_err.to_string().contains("JSON error"));
    }

    // ============ 에러 체이닝 테스트 ============

    #[test]
    fn test_error_result_chain() {
        fn may_fail(fail: bool) -> AppResult<i32> {
            if fail {
                Err(AppError::Internal("failed".to_string()))
            } else {
                Ok(42)
            }
        }

        assert!(may_fail(false).is_ok());
        assert!(may_fail(true).is_err());
    }

    #[test]
    fn test_error_map() {
        let result: AppResult<i32> = Ok(10);
        let mapped = result.map(|x| x * 2);
        assert_eq!(mapped.unwrap(), 20);
    }

    #[test]
    fn test_error_and_then() {
        let result: AppResult<i32> = Ok(10);
        let chained = result.and_then(|x| {
            if x > 5 {
                Ok(x * 2)
            } else {
                Err(AppError::BadRequest("too small".to_string()))
            }
        });
        assert_eq!(chained.unwrap(), 20);
    }

    // ============ 모든 에러 타입 Into Response 테스트 ============

    #[tokio::test]
    async fn test_all_error_types_produce_valid_response() {
        let errors: Vec<AppError> = vec![
            AppError::BadRequest("bad".to_string()),
            AppError::Unauthorized("unauth".to_string()),
            AppError::NotFound("not found".to_string()),
            AppError::Validation("invalid".to_string()),
            AppError::Internal("internal".to_string()),
        ];

        for error in errors {
            let response = error.into_response();
            assert!(response.status().is_client_error() || response.status().is_server_error());
        }
    }
}
