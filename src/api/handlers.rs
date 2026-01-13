//! HTTP request handler module.

use std::borrow::Cow;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::header,
    response::{Html, IntoResponse, Response},
    Json,
};
use axum_extra::extract::CookieJar;
use cookie::Cookie;
use deadpool_redis::redis::AsyncCommands;
use once_cell::sync::Lazy;
use validator::Validate;
use xxhash_rust::xxh3::xxh3_128;

use crate::api::schemas::{validate_short_key, CreateShortUrlRequest, CreateShortUrlResponse};
use crate::api::state::AppState;
use crate::config::APP_CONFIG;
use crate::error::{AppError, AppResult, ValidationErrorExt};
use crate::models::{CreateOrFindResult, NewUrl, UrlCacheData, UrlRepository};
use crate::utils::{gen_rand_str, gen_token, merge_short_key, split_short_key};

/// Index page template.
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {}

/// Redirect page template.
#[derive(Template)]
#[template(path = "redirect.html")]
struct RedirectTemplate {
    object: TemplateUrlData,
}

/// URL data for template rendering.
#[derive(Clone)]
struct TemplateUrlData {
    pub ios_deep_link: String,
    pub ios_fallback_url: String,
    pub android_deep_link: String,
    pub android_fallback_url: String,
    pub default_fallback_url: String,
    pub og_title: String,
    pub og_description: String,
    pub og_image_url: String,
}

impl From<&UrlCacheData> for TemplateUrlData {
    fn from(url: &UrlCacheData) -> Self {
        Self {
            ios_deep_link: url.ios_deep_link.clone().unwrap_or_default(),
            ios_fallback_url: url.ios_fallback_url.clone().unwrap_or_default(),
            android_deep_link: url.android_deep_link.clone().unwrap_or_default(),
            android_fallback_url: url.android_fallback_url.clone().unwrap_or_default(),
            default_fallback_url: url.default_fallback_url.clone(),
            og_title: url.og_title.clone().unwrap_or_default(),
            og_description: url.og_description.clone().unwrap_or_default(),
            og_image_url: url.og_image_url.clone().unwrap_or_default(),
        }
    }
}

/// Pre-rendered index page HTML (static content).
static INDEX_HTML: Lazy<String> = Lazy::new(|| {
    IndexTemplate {}
        .render()
        .expect("Failed to render index template")
});

/// Main page handler.
///
/// Renders the main page and generates a guest token.
///
/// # Route
///
/// `GET /`
pub async fn index_handler(jar: CookieJar) -> AppResult<impl IntoResponse> {
    let token = gen_token("guest").map_err(|e| AppError::Internal(e.to_string()))?;

    let mut cookie_builder = Cookie::build(("token", token))
        .path("/")
        .http_only(true)
        .same_site(cookie::SameSite::Lax);

    // Enable Secure flag in production (HTTPS only)
    if APP_CONFIG.is_production {
        cookie_builder = cookie_builder.secure(true);
    }

    let updated_jar = jar.add(cookie_builder.build());
    Ok((updated_jar, Html(INDEX_HTML.as_str())))
}

/// Short URL creation handler.
///
/// Validates the input URL information and creates a short URL.
/// If the URL already exists, returns the existing short key.
///
/// # Route
///
/// `POST /v1/urls`
pub async fn create_short_url_handler(
    State(state): State<AppState>,
    Json(req_body): Json<CreateShortUrlRequest>,
) -> AppResult<Json<CreateShortUrlResponse>> {
    // 1. Validation
    req_body.validate().map_err(|e| e.to_validation_error())?;

    let default_fallback_url = req_body
        .default_fallback_url
        .as_ref()
        .ok_or_else(|| AppError::Validation("Default fallback URL is required".to_string()))?;

    // 2. Generate hash for duplicate detection using xxHash (fast non-crypto hash)
    let hash_input = format!(
        "{}:{}:{}:{}:{}",
        req_body.ios_deep_link.as_deref().unwrap_or(""),
        req_body.ios_fallback_url.as_deref().unwrap_or(""),
        req_body.android_deep_link.as_deref().unwrap_or(""),
        req_body.android_fallback_url.as_deref().unwrap_or(""),
        default_fallback_url
    );

    let hashed_value = format!("{:032x}", xxh3_128(hash_input.as_bytes()));

    // 3. Prepare new URL data (4-char random key: 2 prefix + 2 suffix)
    let rand_key = gen_rand_str(4);
    let new_url = NewUrl {
        random_key: rand_key,
        ios_deep_link: req_body.ios_deep_link.filter(|s| !s.is_empty()),
        ios_fallback_url: req_body.ios_fallback_url.filter(|s| !s.is_empty()),
        android_deep_link: req_body.android_deep_link.filter(|s| !s.is_empty()),
        android_fallback_url: req_body.android_fallback_url.filter(|s| !s.is_empty()),
        default_fallback_url: default_fallback_url.clone(),
        hashed_value,
        webhook_url: req_body.webhook_url.filter(|s| !s.is_empty()),
        og_title: req_body.og_title.filter(|s| !s.is_empty()),
        og_description: req_body.og_description.filter(|s| !s.is_empty()),
        og_image_url: req_body.og_image_url.filter(|s| !s.is_empty()),
        is_active: true,
    };

    // 4. Create or find existing URL (race-condition safe with ON CONFLICT)
    match UrlRepository::create_or_find(&state.db, &new_url).await? {
        CreateOrFindResult::Created(url) => {
            #[allow(clippy::cast_sign_loss)]
            let short_key = merge_short_key(&url.random_key, url.id as u64);
            Ok(Json(CreateShortUrlResponse::created(short_key)))
        }
        CreateOrFindResult::Existing(url) => {
            #[allow(clippy::cast_sign_loss)]
            let short_key = merge_short_key(&url.random_key, url.id as u64);
            Ok(Json(CreateShortUrlResponse::already_exists_with_key(
                short_key,
            )))
        }
    }
}

/// Short URL redirect handler.
///
/// Takes the short URL key, looks up the original URL information,
/// and renders the redirect page.
///
/// # Route
///
/// `GET /:short_key`
pub async fn redirect_to_original_handler(
    State(state): State<AppState>,
    Path(short_key): Path<String>,
    headers: axum::http::HeaderMap,
) -> AppResult<Response> {
    // 1. Validation
    validate_short_key(&short_key)?;

    // Get user agent for webhook (use Cow to avoid allocation when possible)
    let user_agent: Cow<'static, str> = headers
        .get(header::USER_AGENT)
        .and_then(|h| h.to_str().ok())
        .map_or(Cow::Borrowed("Unknown"), |s| Cow::Owned(s.to_string()));

    // 2. Check cache (MessagePack format for speed)
    let cache_key = format!("urls:{short_key}");
    let mut conn = state
        .cache
        .get()
        .await
        .map_err(|e| AppError::Internal(format!("Redis connection error: {e}")))?;

    if let Ok(cached_val) = conn.get::<_, Vec<u8>>(&cache_key).await {
        if let Ok(url_data) = rmp_serde::from_slice::<UrlCacheData>(&cached_val) {
            // Render page first, then spawn webhook (avoids clone)
            let response = render_redirect_page(&url_data)?;
            url_data.spawn_webhook_task(Cow::Owned(short_key), user_agent);
            return Ok(response);
        }
    }

    // 3. If not in cache, query DB (optimized query)
    let (id, rand_key) = split_short_key(&short_key);
    if id == 0 {
        return Err(AppError::NotFound("URL not found".to_string()));
    }

    #[allow(clippy::cast_possible_wrap)]
    let url_cache_data = UrlRepository::find_by_id_for_cache(&state.db, id as i64)
        .await?
        .ok_or_else(|| AppError::NotFound("URL not found".to_string()))?;

    // Verify random key matches
    if url_cache_data.random_key != rand_key {
        return Err(AppError::NotFound("URL not found".to_string()));
    }

    // 4. Save to cache with MessagePack serialization
    match rmp_serde::to_vec(&url_cache_data) {
        Ok(data) => {
            let cache_result: Result<(), deadpool_redis::redis::RedisError> = conn
                .set_ex(&cache_key, data, APP_CONFIG.cache_ttl_secs)
                .await;

            if let Err(e) = cache_result {
                tracing::error!(
                    cache_key = %cache_key,
                    error = %e,
                    "Failed to cache URL data - DB load may increase"
                );
            }
        }
        Err(e) => {
            tracing::error!(
                cache_key = %cache_key,
                error = %e,
                "Failed to serialize URL data for cache"
            );
        }
    }

    // 5. Render page first, then spawn webhook (avoids clone)
    let response = render_redirect_page(&url_cache_data)?;
    url_cache_data.spawn_webhook_task(Cow::Owned(short_key), user_agent);

    Ok(response)
}

/// Renders the redirect page template.
fn render_redirect_page(url_data: &UrlCacheData) -> AppResult<Response> {
    let template = RedirectTemplate {
        object: TemplateUrlData::from(url_data),
    };

    let html = template.render()?;
    Ok(Html(html).into_response())
}

/// Health check response.
#[derive(serde::Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Liveness probe handler.
///
/// Returns OK if the server is running. Used for Kubernetes liveness probe.
///
/// # Route
///
/// `GET /health`
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness check response.
#[derive(serde::Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub database: &'static str,
    pub cache: &'static str,
}

/// Readiness probe handler.
///
/// Checks database and cache connectivity. Used for Kubernetes readiness probe.
///
/// # Route
///
/// `GET /ready`
pub async fn readiness_handler(
    State(state): State<AppState>,
) -> Result<Json<ReadinessResponse>, (axum::http::StatusCode, Json<ReadinessResponse>)> {
    // Check database connection
    let db_ok = sqlx::query("SELECT 1").fetch_one(&state.db).await.is_ok();

    // Check Redis connection
    let cache_ok = state.cache.get().await.is_ok();

    let response = ReadinessResponse {
        status: if db_ok && cache_ok { "ok" } else { "degraded" },
        database: if db_ok { "connected" } else { "disconnected" },
        cache: if cache_ok {
            "connected"
        } else {
            "disconnected"
        },
    };

    if db_ok && cache_ok {
        Ok(Json(response))
    } else {
        Err((axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(response)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============ TemplateUrlData 테스트 ============

    fn create_test_url_cache_data() -> UrlCacheData {
        UrlCacheData {
            id: 1,
            random_key: "AbXy".to_string(),
            ios_deep_link: Some("app://ios/path".to_string()),
            ios_fallback_url: Some("https://apps.apple.com/app".to_string()),
            android_deep_link: Some("app://android/path".to_string()),
            android_fallback_url: Some("https://play.google.com/app".to_string()),
            default_fallback_url: "https://example.com".to_string(),
            webhook_url: Some("https://webhook.example.com".to_string()),
            og_title: Some("Test Title".to_string()),
            og_description: Some("Test Description".to_string()),
            og_image_url: Some("https://example.com/image.png".to_string()),
            is_active: true,
        }
    }

    fn create_minimal_url_cache_data() -> UrlCacheData {
        UrlCacheData {
            id: 2,
            random_key: "XyZz".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://minimal.com".to_string(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: true,
        }
    }

    #[test]
    fn test_template_url_data_from_full() {
        let cache_data = create_test_url_cache_data();
        let template_data = TemplateUrlData::from(&cache_data);

        assert_eq!(template_data.ios_deep_link, "app://ios/path");
        assert_eq!(template_data.ios_fallback_url, "https://apps.apple.com/app");
        assert_eq!(template_data.android_deep_link, "app://android/path");
        assert_eq!(
            template_data.android_fallback_url,
            "https://play.google.com/app"
        );
        assert_eq!(template_data.default_fallback_url, "https://example.com");
        assert_eq!(template_data.og_title, "Test Title");
        assert_eq!(template_data.og_description, "Test Description");
        assert_eq!(template_data.og_image_url, "https://example.com/image.png");
    }

    #[test]
    fn test_template_url_data_from_minimal() {
        let cache_data = create_minimal_url_cache_data();
        let template_data = TemplateUrlData::from(&cache_data);

        assert!(template_data.ios_deep_link.is_empty());
        assert!(template_data.ios_fallback_url.is_empty());
        assert!(template_data.android_deep_link.is_empty());
        assert!(template_data.android_fallback_url.is_empty());
        assert_eq!(template_data.default_fallback_url, "https://minimal.com");
        assert!(template_data.og_title.is_empty());
        assert!(template_data.og_description.is_empty());
        assert!(template_data.og_image_url.is_empty());
    }

    #[test]
    fn test_template_url_data_clone() {
        let cache_data = create_test_url_cache_data();
        let template_data = TemplateUrlData::from(&cache_data);
        let cloned = template_data.clone();

        assert_eq!(
            template_data.default_fallback_url,
            cloned.default_fallback_url
        );
        assert_eq!(template_data.og_title, cloned.og_title);
    }

    #[test]
    fn test_template_url_data_with_empty_optional_strings() {
        let cache_data = UrlCacheData {
            id: 3,
            random_key: "ZzAa".to_string(),
            ios_deep_link: Some(String::new()),
            ios_fallback_url: Some(String::new()),
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://test.com".to_string(),
            webhook_url: None,
            og_title: Some(String::new()),
            og_description: None,
            og_image_url: None,
            is_active: true,
        };

        let template_data = TemplateUrlData::from(&cache_data);

        // Some(empty_string)은 empty string으로 변환됨
        assert!(template_data.ios_deep_link.is_empty());
        assert!(template_data.ios_fallback_url.is_empty());
        assert!(template_data.og_title.is_empty());
    }

    #[test]
    fn test_template_url_data_with_unicode() {
        let cache_data = UrlCacheData {
            id: 4,
            random_key: "UnIc".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://example.com/한글".to_string(),
            webhook_url: None,
            og_title: Some("한글 제목 🚀".to_string()),
            og_description: Some("日本語の説明".to_string()),
            og_image_url: None,
            is_active: true,
        };

        let template_data = TemplateUrlData::from(&cache_data);

        assert!(template_data.default_fallback_url.contains("한글"));
        assert!(template_data.og_title.contains("🚀"));
        assert!(template_data.og_description.contains("日本語"));
    }

    #[test]
    fn test_template_url_data_with_special_characters() {
        let cache_data = UrlCacheData {
            id: 5,
            random_key: "SpCh".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://example.com/path?param=value&other=123".to_string(),
            webhook_url: None,
            og_title: Some("Title with <script> & \"quotes\"".to_string()),
            og_description: None,
            og_image_url: None,
            is_active: true,
        };

        let template_data = TemplateUrlData::from(&cache_data);

        assert!(template_data.default_fallback_url.contains("param=value"));
        assert!(template_data.og_title.contains("<script>"));
    }

    // ============ INDEX_HTML 테스트 ============

    #[test]
    fn test_index_html_is_not_empty() {
        assert!(!INDEX_HTML.is_empty());
    }

    #[test]
    fn test_index_html_is_valid_html() {
        assert!(INDEX_HTML.contains("<!DOCTYPE html>") || INDEX_HTML.contains("<html"));
    }

    #[test]
    fn test_index_html_contains_body() {
        assert!(INDEX_HTML.contains("<body") || INDEX_HTML.contains("</body>"));
    }

    // ============ render_redirect_page 테스트 ============

    #[test]
    fn test_render_redirect_page_success() {
        let cache_data = create_test_url_cache_data();
        let result = render_redirect_page(&cache_data);
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_redirect_page_minimal() {
        let cache_data = create_minimal_url_cache_data();
        let result = render_redirect_page(&cache_data);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_render_redirect_page_returns_html() {
        use axum::body::to_bytes;

        let cache_data = create_test_url_cache_data();
        let response = render_redirect_page(&cache_data).unwrap();

        let body = to_bytes(response.into_body(), 10240).await.unwrap();
        let html = String::from_utf8_lossy(&body);

        assert!(html.contains("<html") || html.contains("<!DOCTYPE"));
    }

    #[tokio::test]
    async fn test_render_redirect_page_contains_urls() {
        use axum::body::to_bytes;

        let cache_data = create_test_url_cache_data();
        let response = render_redirect_page(&cache_data).unwrap();

        let body = to_bytes(response.into_body(), 10240).await.unwrap();
        let html = String::from_utf8_lossy(&body);

        assert!(html.contains("https://example.com"));
    }

    // ============ CreateShortUrlRequest 해시 생성 로직 테스트 ============

    #[test]
    fn test_hash_input_format() {
        // 핸들러에서 사용하는 해시 입력 형식 테스트
        let ios_deep_link = "app://ios";
        let ios_fallback = "https://apps.apple.com";
        let android_deep_link = "app://android";
        let android_fallback = "https://play.google.com";
        let default_fallback = "https://example.com";

        let hash_input = format!(
            "{}:{}:{}:{}:{}",
            ios_deep_link, ios_fallback, android_deep_link, android_fallback, default_fallback
        );

        assert!(hash_input.contains("app://ios"));
        assert!(hash_input.contains("https://example.com"));
        // 4개의 구분자 (5개 필드를 구분)
        // URL에 포함된 콜론도 포함되므로, 최소 4개의 구분자가 있는지 확인
        assert!(hash_input.matches(':').count() >= 4);
    }

    #[test]
    fn test_hash_input_with_empty_optionals() {
        let hash_input = format!(
            "{}:{}:{}:{}:{}",
            "", // ios_deep_link
            "", // ios_fallback
            "", // android_deep_link
            "", // android_fallback
            "https://example.com"
        );

        assert_eq!(hash_input, "::::https://example.com");
    }

    #[test]
    fn test_xxhash_deterministic() {
        let input = "test_input_for_hash";
        let hash1 = format!("{:032x}", xxh3_128(input.as_bytes()));
        let hash2 = format!("{:032x}", xxh3_128(input.as_bytes()));
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_xxhash_different_inputs() {
        let hash1 = format!("{:032x}", xxh3_128(b"input1"));
        let hash2 = format!("{:032x}", xxh3_128(b"input2"));
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_xxhash_length() {
        let hash = format!("{:032x}", xxh3_128(b"test"));
        assert_eq!(hash.len(), 32); // 128비트 = 32 hex chars
    }

    // ============ Health Check 핸들러 테스트 ============

    #[tokio::test]
    async fn test_health_handler_returns_ok() {
        let response = health_handler().await;
        assert_eq!(response.status, "ok");
    }

    #[test]
    fn test_health_response_has_version() {
        let response = HealthResponse {
            status: "ok",
            version: env!("CARGO_PKG_VERSION"),
        };
        assert!(!response.version.is_empty());
    }

    #[test]
    fn test_health_response_serialize() {
        let response = HealthResponse {
            status: "ok",
            version: "0.1.0",
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("0.1.0"));
    }

    #[test]
    fn test_readiness_response_serialize() {
        let response = ReadinessResponse {
            status: "ok",
            database: "connected",
            cache: "connected",
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("ok"));
        assert!(json.contains("connected"));
    }

    #[test]
    fn test_readiness_response_degraded() {
        let response = ReadinessResponse {
            status: "degraded",
            database: "connected",
            cache: "disconnected",
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("degraded"));
        assert!(json.contains("disconnected"));
    }
}
