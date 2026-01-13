//! URL model module.
//!
//! Contains URL entity, cache data, and repository for database operations.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tokio::sync::Semaphore;

use crate::config::APP_CONFIG;
use crate::error::{AppError, AppResult};

/// Global HTTP client with timeout, connection pooling, and pre-configured headers.
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    let mut default_headers = HeaderMap::new();
    default_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    reqwest::Client::builder()
        .timeout(Duration::from_secs(APP_CONFIG.webhook_timeout_secs))
        .connect_timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(60))
        .default_headers(default_headers)
        .build()
        .expect("Failed to create HTTP client")
});

/// Semaphore to limit concurrent webhook calls.
static WEBHOOK_SEMAPHORE: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(APP_CONFIG.webhook_max_concurrent)));

/// URL model struct that stores shortened URL information.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Url {
    pub id: i64,
    pub random_key: String,
    pub ios_deep_link: Option<String>,
    pub ios_fallback_url: Option<String>,
    pub android_deep_link: Option<String>,
    pub android_fallback_url: Option<String>,
    pub default_fallback_url: String,
    pub hashed_value: String,
    pub webhook_url: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image_url: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// URL data optimized for caching (excludes timestamps for smaller size).
/// Also supports direct database queries with `FromRow`.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UrlCacheData {
    pub id: i64,
    pub random_key: String,
    pub ios_deep_link: Option<String>,
    pub ios_fallback_url: Option<String>,
    pub android_deep_link: Option<String>,
    pub android_fallback_url: Option<String>,
    pub default_fallback_url: String,
    pub webhook_url: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image_url: Option<String>,
    pub is_active: bool,
}

impl From<Url> for UrlCacheData {
    fn from(url: Url) -> Self {
        Self {
            id: url.id,
            random_key: url.random_key,
            ios_deep_link: url.ios_deep_link,
            ios_fallback_url: url.ios_fallback_url,
            android_deep_link: url.android_deep_link,
            android_fallback_url: url.android_fallback_url,
            default_fallback_url: url.default_fallback_url,
            webhook_url: url.webhook_url,
            og_title: url.og_title,
            og_description: url.og_description,
            og_image_url: url.og_image_url,
            is_active: url.is_active,
        }
    }
}

/// Struct for creating a new URL record.
#[derive(Debug, Clone)]
pub struct NewUrl {
    pub random_key: String,
    pub ios_deep_link: Option<String>,
    pub ios_fallback_url: Option<String>,
    pub android_deep_link: Option<String>,
    pub android_fallback_url: Option<String>,
    pub default_fallback_url: String,
    pub hashed_value: String,
    pub webhook_url: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image_url: Option<String>,
    pub is_active: bool,
}

/// Webhook payload sent when URL is accessed.
#[derive(Debug, Serialize)]
struct WebhookPayload {
    short_key: String,
    user_agent: String,
}

impl UrlCacheData {
    /// Spawns an async task to send webhook notification with concurrency control.
    /// Uses `Cow` to avoid unnecessary allocations when possible.
    pub fn spawn_webhook_task(self, short_key: Cow<'static, str>, user_agent: Cow<'static, str>) {
        let semaphore = WEBHOOK_SEMAPHORE.clone();

        tokio::spawn(async move {
            // Try to acquire permit, skip if queue is full
            let Ok(permit) = semaphore.try_acquire() else {
                tracing::warn!(
                    short_key = %short_key,
                    "Webhook queue full, skipping notification"
                );
                return;
            };

            if let Err(e) =
                send_webhook_internal(self.webhook_url.as_ref(), &short_key, &user_agent).await
            {
                tracing::warn!(
                    short_key = %short_key,
                    error = %e,
                    "Webhook request failed"
                );
            }

            drop(permit);
        });
    }
}

/// Internal webhook sending function using the global HTTP client.
async fn send_webhook_internal(
    webhook_url: Option<&String>,
    short_key: &str,
    user_agent: &str,
) -> AppResult<()> {
    let Some(url) = webhook_url.filter(|u| !u.is_empty()) else {
        return Ok(());
    };

    let payload = WebhookPayload {
        short_key: short_key.to_string(),
        user_agent: user_agent.to_string(),
    };

    // Content-Type header is pre-configured in HTTP_CLIENT
    let response = HTTP_CLIENT.post(url).json(&payload).send().await?;

    if !response.status().is_success() {
        tracing::warn!(
            webhook_url = %url,
            status = %response.status().as_u16(),
            "Webhook returned non-success status"
        );
    }

    Ok(())
}

/// Result of create or find operation.
pub enum CreateOrFindResult {
    /// A new URL was created.
    Created(Url),
    /// An existing URL was found.
    Existing(Url),
}

/// URL repository for database operations.
pub struct UrlRepository;

impl UrlRepository {
    /// Finds an existing URL by its hash value.
    /// Returns the URL if it exists and is not deleted.
    pub async fn find_by_hashed_value(
        pool: &sqlx::PgPool,
        hashed_value: &str,
    ) -> AppResult<Option<Url>> {
        let url = sqlx::query_as::<_, Url>(
            r"
            SELECT id, random_key, ios_deep_link, ios_fallback_url,
                   android_deep_link, android_fallback_url, default_fallback_url,
                   hashed_value, webhook_url, og_title, og_description,
                   og_image_url, is_active, created_at, updated_at, deleted_at
            FROM urls
            WHERE hashed_value = $1 AND deleted_at IS NULL
            LIMIT 1
            ",
        )
        .bind(hashed_value)
        .fetch_optional(pool)
        .await?;

        Ok(url)
    }

    /// Finds a URL by its ID and returns only cache-relevant fields.
    /// Optimized query that excludes timestamps for better performance.
    pub async fn find_by_id_for_cache(
        pool: &sqlx::PgPool,
        id: i64,
    ) -> AppResult<Option<UrlCacheData>> {
        let url = sqlx::query_as::<_, UrlCacheData>(
            r"
            SELECT id, random_key, ios_deep_link, ios_fallback_url,
                   android_deep_link, android_fallback_url, default_fallback_url,
                   webhook_url, og_title, og_description, og_image_url, is_active
            FROM urls
            WHERE id = $1 AND deleted_at IS NULL AND is_active = true
            LIMIT 1
            ",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(url)
    }

    /// Creates a new URL record or returns existing one if hash already exists.
    /// This prevents race conditions using ON CONFLICT.
    pub async fn create_or_find(
        pool: &sqlx::PgPool,
        new_url: &NewUrl,
    ) -> AppResult<CreateOrFindResult> {
        // First, try to insert. If conflict on hashed_value, do nothing.
        let insert_result = sqlx::query_as::<_, Url>(
            r"
            INSERT INTO urls (
                random_key, ios_deep_link, ios_fallback_url,
                android_deep_link, android_fallback_url, default_fallback_url,
                hashed_value, webhook_url, og_title, og_description,
                og_image_url, is_active, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW(), NOW())
            ON CONFLICT (hashed_value) WHERE deleted_at IS NULL
            DO NOTHING
            RETURNING id, random_key, ios_deep_link, ios_fallback_url,
                      android_deep_link, android_fallback_url, default_fallback_url,
                      hashed_value, webhook_url, og_title, og_description,
                      og_image_url, is_active, created_at, updated_at, deleted_at
            ",
        )
        .bind(&new_url.random_key)
        .bind(&new_url.ios_deep_link)
        .bind(&new_url.ios_fallback_url)
        .bind(&new_url.android_deep_link)
        .bind(&new_url.android_fallback_url)
        .bind(&new_url.default_fallback_url)
        .bind(&new_url.hashed_value)
        .bind(&new_url.webhook_url)
        .bind(&new_url.og_title)
        .bind(&new_url.og_description)
        .bind(&new_url.og_image_url)
        .bind(new_url.is_active)
        .fetch_optional(pool)
        .await?;

        if let Some(url) = insert_result {
            return Ok(CreateOrFindResult::Created(url));
        }

        // Insert returned nothing (conflict), find the existing record
        let existing = Self::find_by_hashed_value(pool, &new_url.hashed_value)
            .await?
            .ok_or_else(|| {
                AppError::Internal("Race condition: URL not found after conflict".to_string())
            })?;

        Ok(CreateOrFindResult::Existing(existing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_url() -> Url {
        Url {
            id: 1,
            random_key: "AbXy".to_string(),
            ios_deep_link: Some("app://ios".to_string()),
            ios_fallback_url: Some("https://apps.apple.com".to_string()),
            android_deep_link: Some("app://android".to_string()),
            android_fallback_url: Some("https://play.google.com".to_string()),
            default_fallback_url: "https://example.com".to_string(),
            hashed_value: "abc123hash".to_string(),
            webhook_url: Some("https://webhook.example.com".to_string()),
            og_title: Some("Test Title".to_string()),
            og_description: Some("Test Description".to_string()),
            og_image_url: Some("https://example.com/image.png".to_string()),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }

    fn create_minimal_url() -> Url {
        Url {
            id: 2,
            random_key: "XyZz".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://minimal.com".to_string(),
            hashed_value: "minimal123".to_string(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        }
    }

    // ============ Url 구조체 테스트 ============

    #[test]
    fn test_url_clone() {
        let url = create_test_url();
        let cloned = url.clone();
        assert_eq!(url.id, cloned.id);
        assert_eq!(url.random_key, cloned.random_key);
        assert_eq!(url.default_fallback_url, cloned.default_fallback_url);
    }

    #[test]
    fn test_url_debug() {
        let url = create_test_url();
        let debug_str = format!("{url:?}");
        assert!(debug_str.contains("Url"));
        assert!(debug_str.contains("random_key"));
    }

    #[test]
    fn test_url_serialize() {
        let url = create_test_url();
        let json = serde_json::to_string(&url).unwrap();
        assert!(json.contains("example.com"));
        assert!(json.contains("random_key"));
    }

    #[test]
    fn test_url_deserialize() {
        let url = create_test_url();
        let json = serde_json::to_string(&url).unwrap();
        let deserialized: Url = serde_json::from_str(&json).unwrap();
        assert_eq!(url.id, deserialized.id);
        assert_eq!(url.default_fallback_url, deserialized.default_fallback_url);
    }

    #[test]
    fn test_url_with_deleted_at() {
        let mut url = create_test_url();
        url.deleted_at = Some(chrono::Utc::now());
        let json = serde_json::to_string(&url).unwrap();
        assert!(json.contains("deleted_at"));
    }

    // ============ UrlCacheData 구조체 테스트 ============

    #[test]
    fn test_url_cache_data_from_url() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.clone().into();

        assert_eq!(cache_data.id, url.id);
        assert_eq!(cache_data.random_key, url.random_key);
        assert_eq!(cache_data.ios_deep_link, url.ios_deep_link);
        assert_eq!(cache_data.default_fallback_url, url.default_fallback_url);
        assert_eq!(cache_data.is_active, url.is_active);
    }

    #[test]
    fn test_url_cache_data_from_minimal_url() {
        let url = create_minimal_url();
        let cache_data: UrlCacheData = url.clone().into();

        assert_eq!(cache_data.id, url.id);
        assert!(cache_data.ios_deep_link.is_none());
        assert!(cache_data.webhook_url.is_none());
        assert!(!cache_data.is_active);
    }

    #[test]
    fn test_url_cache_data_clone() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.into();
        let cloned = cache_data.clone();

        assert_eq!(cache_data.id, cloned.id);
        assert_eq!(cache_data.random_key, cloned.random_key);
    }

    #[test]
    fn test_url_cache_data_serialize() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.into();
        let json = serde_json::to_string(&cache_data).unwrap();

        assert!(json.contains("id"));
        assert!(json.contains("random_key"));
        assert!(json.contains("default_fallback_url"));
        // created_at, updated_at, deleted_at는 제외되어야 함
        assert!(!json.contains("created_at"));
        assert!(!json.contains("updated_at"));
        assert!(!json.contains("deleted_at"));
    }

    #[test]
    fn test_url_cache_data_deserialize() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.into();
        let json = serde_json::to_string(&cache_data).unwrap();
        let deserialized: UrlCacheData = serde_json::from_str(&json).unwrap();

        assert_eq!(cache_data.id, deserialized.id);
        assert_eq!(cache_data.random_key, deserialized.random_key);
    }

    #[test]
    fn test_url_cache_data_debug() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.into();
        let debug_str = format!("{cache_data:?}");
        assert!(debug_str.contains("UrlCacheData"));
    }

    // ============ NewUrl 구조체 테스트 ============

    #[test]
    fn test_new_url_create() {
        let new_url = NewUrl {
            random_key: "AbXy".to_string(),
            ios_deep_link: Some("app://ios".to_string()),
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://example.com".to_string(),
            hashed_value: "hash123".to_string(),
            webhook_url: None,
            og_title: Some("Title".to_string()),
            og_description: None,
            og_image_url: None,
            is_active: true,
        };

        assert_eq!(new_url.random_key, "AbXy");
        assert!(new_url.is_active);
    }

    #[test]
    fn test_new_url_clone() {
        let new_url = NewUrl {
            random_key: "XyZz".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://test.com".to_string(),
            hashed_value: "testhash".to_string(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: false,
        };

        let cloned = new_url.clone();
        assert_eq!(new_url.random_key, cloned.random_key);
        assert_eq!(new_url.hashed_value, cloned.hashed_value);
    }

    #[test]
    fn test_new_url_debug() {
        let new_url = NewUrl {
            random_key: "ZzAa".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://debug.com".to_string(),
            hashed_value: "debughash".to_string(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: true,
        };

        let debug_str = format!("{new_url:?}");
        assert!(debug_str.contains("NewUrl"));
        assert!(debug_str.contains("random_key"));
    }

    // ============ WebhookPayload 테스트 (private이므로 간접 테스트) ============

    #[test]
    fn test_url_cache_data_with_webhook() {
        let url = create_test_url();
        let cache_data: UrlCacheData = url.into();
        assert!(cache_data.webhook_url.is_some());
        assert_eq!(
            cache_data.webhook_url.as_ref().unwrap(),
            "https://webhook.example.com"
        );
    }

    #[test]
    fn test_url_cache_data_without_webhook() {
        let url = create_minimal_url();
        let cache_data: UrlCacheData = url.into();
        assert!(cache_data.webhook_url.is_none());
    }

    // ============ 직렬화/역직렬화 왕복 테스트 ============

    #[test]
    fn test_url_roundtrip_serialization() {
        let original = create_test_url();
        let json = serde_json::to_string(&original).unwrap();
        let restored: Url = serde_json::from_str(&json).unwrap();

        assert_eq!(original.id, restored.id);
        assert_eq!(original.random_key, restored.random_key);
        assert_eq!(original.ios_deep_link, restored.ios_deep_link);
        assert_eq!(original.android_deep_link, restored.android_deep_link);
        assert_eq!(original.default_fallback_url, restored.default_fallback_url);
        assert_eq!(original.hashed_value, restored.hashed_value);
        assert_eq!(original.webhook_url, restored.webhook_url);
        assert_eq!(original.og_title, restored.og_title);
        assert_eq!(original.og_description, restored.og_description);
        assert_eq!(original.og_image_url, restored.og_image_url);
        assert_eq!(original.is_active, restored.is_active);
    }

    #[test]
    fn test_url_cache_data_roundtrip_serialization() {
        let url = create_test_url();
        let original: UrlCacheData = url.into();
        let json = serde_json::to_string(&original).unwrap();
        let restored: UrlCacheData = serde_json::from_str(&json).unwrap();

        assert_eq!(original.id, restored.id);
        assert_eq!(original.random_key, restored.random_key);
        assert_eq!(original.default_fallback_url, restored.default_fallback_url);
    }

    // ============ 엣지 케이스 테스트 ============

    #[test]
    fn test_url_with_empty_strings() {
        let url = Url {
            id: 100,
            random_key: String::new(),
            ios_deep_link: Some(String::new()),
            ios_fallback_url: Some(String::new()),
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: String::new(),
            hashed_value: String::new(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        let cache_data: UrlCacheData = url.into();
        assert!(cache_data.random_key.is_empty());
        assert_eq!(cache_data.ios_deep_link, Some(String::new()));
    }

    #[test]
    fn test_url_with_unicode() {
        let url = Url {
            id: 200,
            random_key: "AaBb".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://example.com/한글".to_string(),
            hashed_value: "유니코드해시".to_string(),
            webhook_url: None,
            og_title: Some("한글 제목 🚀".to_string()),
            og_description: Some("テスト説明".to_string()),
            og_image_url: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        let json = serde_json::to_string(&url).unwrap();
        let restored: Url = serde_json::from_str(&json).unwrap();

        assert_eq!(url.og_title, restored.og_title);
        assert_eq!(url.og_description, restored.og_description);
    }

    #[test]
    fn test_url_with_large_id() {
        let url = Url {
            id: i64::MAX,
            random_key: "BbCc".to_string(),
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: "https://large-id.com".to_string(),
            hashed_value: "largeidhash".to_string(),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };

        let cache_data: UrlCacheData = url.into();
        assert_eq!(cache_data.id, i64::MAX);
    }
}
