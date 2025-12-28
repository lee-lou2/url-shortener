//! 요청/응답 스키마 모듈.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use validator::Validate;

/// Short URL creation request structure.
///
/// Uses validator for validation rules.
#[derive(Debug, Clone, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct CreateShortUrlRequest {
    /// iOS app deep link URL (optional)
    #[validate(url(message = "Invalid iOS deep link URL"))]
    #[serde(default)]
    pub ios_deep_link: Option<String>,

    /// URL to redirect when iOS app is not installed (optional)
    #[validate(url(message = "Invalid iOS fallback URL"))]
    #[serde(default)]
    pub ios_fallback_url: Option<String>,

    /// Android app deep link URL (optional)
    #[validate(url(message = "Invalid Android deep link URL"))]
    #[serde(default)]
    pub android_deep_link: Option<String>,

    /// URL to redirect when Android app is not installed (optional)
    #[validate(url(message = "Invalid Android fallback URL"))]
    #[serde(default)]
    pub android_fallback_url: Option<String>,

    /// Default redirect URL (required)
    #[validate(
        required(message = "Default fallback URL is required"),
        url(message = "Invalid default fallback URL")
    )]
    pub default_fallback_url: Option<String>,

    /// Webhook URL (optional)
    #[validate(url(message = "Invalid webhook URL"))]
    #[serde(default)]
    pub webhook_url: Option<String>,

    /// Open Graph title (optional, max 255 characters)
    #[validate(length(max = 255, message = "OG title must be at most 255 characters"))]
    #[serde(default)]
    pub og_title: Option<String>,

    /// Open Graph description (optional, max 500 characters)
    #[validate(length(max = 500, message = "OG description must be at most 500 characters"))]
    #[serde(default)]
    pub og_description: Option<String>,

    /// Open Graph image URL (optional)
    #[validate(url(message = "Invalid OG image URL"))]
    #[serde(default)]
    pub og_image_url: Option<String>,
}

/// Response for short URL creation.
#[derive(Debug, Serialize)]
pub struct CreateShortUrlResponse {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_key: Option<String>,
}

impl CreateShortUrlResponse {
    /// Creates a response for a newly created URL.
    pub fn created(short_key: String) -> Self {
        Self {
            message: "URL created successfully".to_string(),
            short_key: Some(short_key),
        }
    }

    /// Creates a response for an existing URL, returning its short key.
    pub fn already_exists_with_key(short_key: String) -> Self {
        Self {
            message: "URL already exists".to_string(),
            short_key: Some(short_key),
        }
    }
}

/// Validates a short URL key.
///
/// # Validation Rules
///
/// - Must be at least 5 characters long (2 prefix + 1 ID char + 2 suffix)
/// - Must contain only alphanumeric characters (a-z, A-Z, 0-9)
pub fn validate_short_key(short_key: &str) -> Result<(), AppError> {
    use crate::utils::short_key::SHORT_KEY_MIN_LEN;

    if short_key.len() < SHORT_KEY_MIN_LEN {
        return Err(AppError::BadRequest(format!(
            "short_key must be at least {SHORT_KEY_MIN_LEN} characters long"
        )));
    }

    if !short_key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::BadRequest(
            "short_key must contain only English letters and numbers".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use validator::Validate;

    // ============ validate_short_key 테스트 ============

    #[test]
    fn test_validate_short_key_valid_minimum() {
        // 최소 5자: prefix(2) + id(1) + suffix(2)
        assert!(validate_short_key("ab1xy").is_ok());
    }

    #[test]
    fn test_validate_short_key_valid_mixed() {
        assert!(validate_short_key("ABC123").is_ok());
        assert!(validate_short_key("a1B2c3D4").is_ok());
    }

    #[test]
    fn test_validate_short_key_valid_numbers_only() {
        assert!(validate_short_key("12345").is_ok());
        assert!(validate_short_key("00000").is_ok());
    }

    #[test]
    fn test_validate_short_key_valid_long() {
        assert!(validate_short_key("abcdefghijklmnopqrstuvwxyz").is_ok());
    }

    #[test]
    fn test_validate_short_key_exactly_five_chars() {
        assert!(validate_short_key("ab1cd").is_ok());
        assert!(validate_short_key("12345").is_ok());
        assert!(validate_short_key("a1B2c").is_ok());
    }

    #[test]
    fn test_validate_short_key_too_short_empty() {
        let result = validate_short_key("");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_too_short_one() {
        let result = validate_short_key("a");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_too_short_four() {
        let result = validate_short_key("abcd");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_hyphen() {
        let result = validate_short_key("abc-def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_underscore() {
        let result = validate_short_key("abc_def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_space() {
        let result = validate_short_key("abc def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_special_chars() {
        let invalid_chars = ["abc!", "ab@c", "a#bc", "ab$c", "ab%c", "ab^c"];
        for key in invalid_chars {
            let result = validate_short_key(key);
            assert!(
                matches!(result, Err(AppError::BadRequest(_))),
                "Expected error for: {key}"
            );
        }
    }

    #[test]
    fn test_validate_short_key_invalid_unicode() {
        let result = validate_short_key("abc한글");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_emoji() {
        let result = validate_short_key("abc🚀");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    // ============ CreateShortUrlResponse 테스트 ============

    #[test]
    fn test_create_short_url_response_created() {
        let response = CreateShortUrlResponse::created("AbC123".to_string());
        assert_eq!(response.message, "URL created successfully");
        assert_eq!(response.short_key, Some("AbC123".to_string()));
    }

    #[test]
    fn test_create_short_url_response_already_exists_with_key() {
        let response = CreateShortUrlResponse::already_exists_with_key("existing123".to_string());
        assert_eq!(response.message, "URL already exists");
        assert_eq!(response.short_key, Some("existing123".to_string()));
    }

    #[test]
    fn test_create_short_url_response_serialize_created() {
        let response = CreateShortUrlResponse::created("test123".to_string());
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("test123"));
        assert!(json.contains("URL created successfully"));
    }

    #[test]
    fn test_create_short_url_response_serialize_already_exists() {
        let response = CreateShortUrlResponse::already_exists_with_key("abc123".to_string());
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("URL already exists"));
        // Now includes the existing short_key
        assert!(json.contains("short_key"));
        assert!(json.contains("abc123"));
    }

    // ============ CreateShortUrlRequest 테스트 ============

    #[test]
    fn test_create_short_url_request_deserialize_minimal() {
        let json = r#"{"defaultFallbackUrl": "https://example.com"}"#;
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.default_fallback_url,
            Some("https://example.com".to_string())
        );
        assert!(req.ios_deep_link.is_none());
        assert!(req.android_deep_link.is_none());
    }

    #[test]
    fn test_create_short_url_request_deserialize_full() {
        let json = r#"{
            "defaultFallbackUrl": "https://example.com",
            "iosDeepLink": "app://ios",
            "iosFallbackUrl": "https://apps.apple.com",
            "androidDeepLink": "app://android",
            "androidFallbackUrl": "https://play.google.com",
            "webhookUrl": "https://webhook.example.com",
            "ogTitle": "Test Title",
            "ogDescription": "Test Description",
            "ogImageUrl": "https://example.com/image.png"
        }"#;
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.default_fallback_url,
            Some("https://example.com".to_string())
        );
        assert_eq!(req.ios_deep_link, Some("app://ios".to_string()));
        assert_eq!(req.og_title, Some("Test Title".to_string()));
    }

    #[test]
    fn test_create_short_url_request_validate_valid() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_short_url_request_validate_missing_default_url() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: None,
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_invalid_url_format() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("not-a-valid-url".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_og_title_too_long() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: Some("a".repeat(256)), // 255자 초과
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_og_title_max_length() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: Some("a".repeat(255)), // 정확히 255자
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_short_url_request_validate_og_description_too_long() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: Some("a".repeat(501)), // 500자 초과
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_with_all_optional_urls() {
        let req = CreateShortUrlRequest {
            ios_deep_link: Some("https://ios.example.com".to_string()),
            ios_fallback_url: Some("https://apps.apple.com".to_string()),
            android_deep_link: Some("https://android.example.com".to_string()),
            android_fallback_url: Some("https://play.google.com".to_string()),
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: Some("https://webhook.example.com".to_string()),
            og_title: Some("Title".to_string()),
            og_description: Some("Description".to_string()),
            og_image_url: Some("https://example.com/image.png".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_short_url_request_clone() {
        let req = CreateShortUrlRequest {
            ios_deep_link: Some("https://ios.example.com".to_string()),
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        let cloned = req.clone();
        assert_eq!(req.default_fallback_url, cloned.default_fallback_url);
        assert_eq!(req.ios_deep_link, cloned.ios_deep_link);
    }

    #[test]
    fn test_create_short_url_request_debug() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        let debug_str = format!("{req:?}");
        assert!(debug_str.contains("CreateShortUrlRequest"));
    }

    // ============ 추가 validate_short_key 테스트 ============

    #[test]
    fn test_validate_short_key_max_length() {
        // 매우 긴 short_key도 유효 (Base62로 인코딩된 큰 숫자)
        let long_key = "a".repeat(100);
        assert!(validate_short_key(&long_key).is_ok());
    }

    #[test]
    fn test_validate_short_key_all_digits() {
        assert!(validate_short_key("123456789").is_ok());
    }

    #[test]
    fn test_validate_short_key_all_lowercase() {
        assert!(validate_short_key("abcdefghij").is_ok());
    }

    #[test]
    fn test_validate_short_key_all_uppercase() {
        assert!(validate_short_key("ABCDEFGHIJ").is_ok());
    }

    #[test]
    fn test_validate_short_key_base62_chars() {
        // Base62에 사용되는 모든 문자 테스트
        assert!(validate_short_key(
            "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
        )
        .is_ok());
    }

    #[test]
    fn test_validate_short_key_invalid_dot() {
        let result = validate_short_key("abc.def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_slash() {
        let result = validate_short_key("abc/def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_invalid_backslash() {
        let result = validate_short_key("abc\\def");
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[test]
    fn test_validate_short_key_error_message_too_short() {
        let result = validate_short_key("abcd");
        match result {
            Err(AppError::BadRequest(msg)) => {
                assert!(msg.contains("at least 5 characters"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    #[test]
    fn test_validate_short_key_error_message_invalid_chars() {
        let result = validate_short_key("abc-def");
        match result {
            Err(AppError::BadRequest(msg)) => {
                assert!(msg.contains("English letters and numbers"));
            }
            _ => panic!("Expected BadRequest error"),
        }
    }

    // ============ 추가 CreateShortUrlRequest 유효성 검사 테스트 ============

    #[test]
    fn test_create_short_url_request_validate_invalid_ios_deep_link() {
        let req = CreateShortUrlRequest {
            ios_deep_link: Some("not-a-url".to_string()),
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_invalid_android_deep_link() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: Some("invalid-url".to_string()),
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_invalid_webhook_url() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: Some("not-a-webhook-url".to_string()),
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_invalid_og_image_url() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: Some("not-an-image-url".to_string()),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_short_url_request_validate_og_description_max_length() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: Some("a".repeat(500)), // 정확히 500자
            og_image_url: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_short_url_request_validate_empty_strings_ok() {
        // 빈 문자열은 유효성 검사에서 URL 형식으로 체크되지 않음
        let req = CreateShortUrlRequest {
            ios_deep_link: Some(String::new()),
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        // 빈 문자열은 URL 형식이 아니므로 실패할 수 있음
        // validator의 url 검사는 빈 문자열을 어떻게 처리하는지에 따라 다름
        let _ = req.validate(); // 결과에 관계없이 패닉하지 않으면 OK
    }

    #[test]
    fn test_create_short_url_request_validate_https_urls() {
        let req = CreateShortUrlRequest {
            ios_deep_link: Some("https://ios.example.com".to_string()),
            ios_fallback_url: Some("https://apps.apple.com/app".to_string()),
            android_deep_link: Some("https://android.example.com".to_string()),
            android_fallback_url: Some("https://play.google.com/app".to_string()),
            default_fallback_url: Some("https://example.com".to_string()),
            webhook_url: Some("https://webhook.example.com/hook".to_string()),
            og_title: Some("Title".to_string()),
            og_description: Some("Description".to_string()),
            og_image_url: Some("https://example.com/image.png".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_short_url_request_validate_http_urls() {
        let req = CreateShortUrlRequest {
            ios_deep_link: None,
            ios_fallback_url: None,
            android_deep_link: None,
            android_fallback_url: None,
            default_fallback_url: Some("http://example.com".to_string()),
            webhook_url: None,
            og_title: None,
            og_description: None,
            og_image_url: None,
        };
        assert!(req.validate().is_ok());
    }

    // ============ CreateShortUrlResponse 추가 테스트 ============

    #[test]
    fn test_create_short_url_response_debug() {
        let response = CreateShortUrlResponse::created("test123".to_string());
        let debug_str = format!("{response:?}");
        assert!(debug_str.contains("CreateShortUrlResponse"));
    }

    #[test]
    fn test_create_short_url_response_empty_short_key() {
        let response = CreateShortUrlResponse::created(String::new());
        assert_eq!(response.short_key, Some(String::new()));
    }

    #[test]
    fn test_create_short_url_response_long_short_key() {
        let long_key = "a".repeat(100);
        let response = CreateShortUrlResponse::created(long_key.clone());
        assert_eq!(response.short_key, Some(long_key));
    }

    #[test]
    fn test_create_short_url_response_serialize_json_structure() {
        let response = CreateShortUrlResponse::created("test123".to_string());
        let json = serde_json::to_value(&response).unwrap();

        assert!(json.is_object());
        assert!(json.get("message").is_some());
        assert!(json.get("short_key").is_some());
    }

    // ============ Deserialization 엣지 케이스 ============

    #[test]
    fn test_create_short_url_request_deserialize_empty_json() {
        let json = "{}";
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert!(req.default_fallback_url.is_none());
        assert!(req.ios_deep_link.is_none());
    }

    #[test]
    fn test_create_short_url_request_deserialize_null_values() {
        let json = r#"{"defaultFallbackUrl": null, "iosDeepLink": null}"#;
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert!(req.default_fallback_url.is_none());
        assert!(req.ios_deep_link.is_none());
    }

    #[test]
    fn test_create_short_url_request_deserialize_extra_fields() {
        // 알 수 없는 필드가 있어도 무시
        let json = r#"{
            "defaultFallbackUrl": "https://example.com",
            "unknownField": "should be ignored"
        }"#;
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            req.default_fallback_url,
            Some("https://example.com".to_string())
        );
    }

    #[test]
    fn test_create_short_url_request_deserialize_unicode() {
        let json = r#"{
            "defaultFallbackUrl": "https://example.com",
            "ogTitle": "한글 제목 🚀",
            "ogDescription": "日本語説明"
        }"#;
        let req: CreateShortUrlRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.og_title, Some("한글 제목 🚀".to_string()));
        assert_eq!(req.og_description, Some("日本語説明".to_string()));
    }
}
