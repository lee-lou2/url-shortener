//! Middleware module.
//!
//! Provides authentication and other request processing middleware.

use axum::{body::Body, extract::Request, http::header, middleware::Next, response::Response};
use axum_extra::extract::CookieJar;

use crate::error::AppError;
use crate::utils::{parse_token, Claims};

/// Extension type for storing authenticated user claims.
/// Can be extracted in handlers via axum's Extension extractor.
#[derive(Clone)]
#[allow(dead_code)]
pub struct AuthUser(pub Claims);

/// JWT Authentication Middleware.
///
/// Validates the Authorization header or cookie token to verify JWT validity.
/// If the token is valid, stores the user information in request extensions.
///
/// # Authentication Header Format
///
/// `Authorization: Bearer <token>`
///
/// # Process
///
/// 1. Check for Authorization header with Bearer schema
/// 2. If not found, check for token in cookies
/// 3. Parse and validate the JWT token
/// 4. Store user claims in request extensions
///
/// # Error Responses
///
/// - 401 Unauthorized: When no token is provided or token is invalid
pub async fn jwt_auth(
    jar: CookieJar,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = extract_token(&request, &jar);

    let Some(token) = token else {
        return Err(AppError::Unauthorized("No token provided".to_string()));
    };

    match parse_token(&token) {
        Ok(claims) => {
            request.extensions_mut().insert(AuthUser(claims));
            Ok(next.run(request).await)
        }
        Err(e) => Err(AppError::Unauthorized(e.to_string())),
    }
}

/// Extracts the JWT token from the request.
///
/// First checks the Authorization header for a Bearer token,
/// then falls back to checking cookies.
fn extract_token(request: &Request<Body>, jar: &CookieJar) -> Option<String> {
    // Try Authorization header first
    if let Some(auth_header) = request.headers().get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ") {
                return Some(token.to_string());
            }
        }
    }

    // Fall back to cookie
    jar.get("token").map(|c| c.value().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request};
    use axum_extra::extract::cookie::Cookie;

    // ============ extract_token 함수 테스트 ============

    #[test]
    fn test_extract_token_from_bearer_header() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Bearer test_token_123")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert_eq!(token, Some("test_token_123".to_string()));
    }

    #[test]
    fn test_extract_token_from_bearer_header_with_spaces() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Bearer   token_with_leading_space")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        // Bearer 다음 공백이 있으면 그대로 포함됨
        assert_eq!(token, Some("  token_with_leading_space".to_string()));
    }

    #[test]
    fn test_extract_token_no_bearer_prefix() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Basic abc123")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert!(token.is_none());
    }

    #[test]
    fn test_extract_token_empty_bearer() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Bearer ")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert_eq!(token, Some(String::new()));
    }

    #[test]
    fn test_extract_token_from_cookie() {
        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let cookie = Cookie::new("token", "cookie_token_456");
        let jar = CookieJar::new().add(cookie);
        let token = extract_token(&request, &jar);

        assert_eq!(token, Some("cookie_token_456".to_string()));
    }

    #[test]
    fn test_extract_token_header_takes_precedence() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Bearer header_token")
            .body(Body::empty())
            .unwrap();

        let cookie = Cookie::new("token", "cookie_token");
        let jar = CookieJar::new().add(cookie);
        let token = extract_token(&request, &jar);

        // Authorization 헤더가 쿠키보다 우선
        assert_eq!(token, Some("header_token".to_string()));
    }

    #[test]
    fn test_extract_token_no_token() {
        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert!(token.is_none());
    }

    #[test]
    fn test_extract_token_wrong_cookie_name() {
        let request = Request::builder().uri("/test").body(Body::empty()).unwrap();

        let cookie = Cookie::new("auth_token", "wrong_cookie_name");
        let jar = CookieJar::new().add(cookie);
        let token = extract_token(&request, &jar);

        assert!(token.is_none());
    }

    #[test]
    fn test_extract_token_case_sensitive_bearer() {
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "bearer lowercase_token")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        // "Bearer"는 대소문자를 구분함
        assert!(token.is_none());
    }

    #[test]
    fn test_extract_token_jwt_format() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ0ZXN0In0.signature";
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, format!("Bearer {jwt}"))
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert_eq!(token, Some(jwt.to_string()));
    }

    #[test]
    fn test_extract_token_unicode_token() {
        // 유니코드는 HTTP 헤더에서 직접 지원되지 않으므로
        // ASCII 토큰만 테스트
        let request = Request::builder()
            .uri("/test")
            .header(header::AUTHORIZATION, "Bearer unicode_test_token_123")
            .body(Body::empty())
            .unwrap();

        let jar = CookieJar::new();
        let token = extract_token(&request, &jar);

        assert_eq!(token, Some("unicode_test_token_123".to_string()));
    }

    // ============ AuthUser 구조체 테스트 ============

    #[test]
    fn test_auth_user_clone() {
        use crate::utils::Claims;

        let claims = Claims {
            sub: "test_user".to_string(),
            exp: 9999999999,
            iat: 1000000000,
        };

        let auth_user = AuthUser(claims.clone());
        let cloned = auth_user.clone();

        assert_eq!(auth_user.0.sub, cloned.0.sub);
        assert_eq!(auth_user.0.exp, cloned.0.exp);
    }
}
