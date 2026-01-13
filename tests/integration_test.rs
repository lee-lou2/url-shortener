//! Integration test module.
//!
//! Contains end-to-end tests for the URL shortening service.

use url_shortener::api::schemas::{
    validate_short_key, CreateShortUrlRequest, CreateShortUrlResponse,
};
use url_shortener::error::{AppError, AppResult};
use url_shortener::models::{NewUrl, Url, UrlCacheData};
use url_shortener::utils::{
    gen_rand_str, gen_token, merge_short_key, parse_token, split_short_key,
};
use validator::Validate;

// ============ 전체 흐름 통합 테스트 ============

/// URL 단축 전체 흐름 테스트 (DB 없이)
#[test]
fn test_url_shortening_flow_without_db() {
    // 1. 요청 데이터 생성
    let req = CreateShortUrlRequest {
        ios_deep_link: Some("https://ios.example.com".to_string()),
        ios_fallback_url: Some("https://apps.apple.com".to_string()),
        android_deep_link: Some("https://android.example.com".to_string()),
        android_fallback_url: Some("https://play.google.com".to_string()),
        default_fallback_url: Some("https://example.com".to_string()),
        webhook_url: Some("https://webhook.example.com".to_string()),
        og_title: Some("Test Title".to_string()),
        og_description: Some("Test Description".to_string()),
        og_image_url: Some("https://example.com/image.png".to_string()),
    };

    // 2. 유효성 검사
    assert!(req.validate().is_ok());

    // 3. 랜덤 키 생성 (4자: 앞 2자 + 뒤 2자)
    let rand_key = gen_rand_str(4);
    assert_eq!(rand_key.len(), 4);
    assert!(rand_key.chars().all(|c| c.is_ascii_alphanumeric()));

    // 4. Short key 생성 시뮬레이션
    let simulated_id: u64 = 12345;
    let short_key = merge_short_key(&rand_key, simulated_id);

    // 5. Short key 유효성 검사
    assert!(validate_short_key(&short_key).is_ok());

    // 6. Short key 디코딩
    let (decoded_id, decoded_rand) = split_short_key(&short_key);
    assert_eq!(decoded_id, simulated_id);
    assert_eq!(decoded_rand, rand_key);
}

/// JWT 인증 흐름 테스트
#[test]
fn test_jwt_authentication_flow() {
    // 1. 토큰 생성
    let subject = "test_user_123";
    let token = gen_token(subject).expect("Failed to generate token");

    // 2. 토큰 구조 확인
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);

    // 3. 토큰 파싱
    let claims = parse_token(&token).expect("Failed to parse token");
    assert_eq!(claims.sub, subject);

    // 4. 만료 시간 확인
    assert!(claims.exp > claims.iat);
}

/// Short key 생성 및 검증 통합 테스트
#[test]
fn test_short_key_generation_and_validation() {
    for id in [1, 100, 1000, 10000, 100_000, 1_000_000] {
        let rand_key = gen_rand_str(4);
        let short_key = merge_short_key(&rand_key, id);

        // 유효성 검사
        assert!(
            validate_short_key(&short_key).is_ok(),
            "Failed for id: {id}, short_key: {short_key}"
        );

        // 디코딩
        let (decoded_id, decoded_rand) = split_short_key(&short_key);
        assert_eq!(decoded_id, id);
        assert_eq!(decoded_rand, rand_key);
    }
}

// ============ 에러 처리 통합 테스트 ============

#[test]
fn test_error_handling_chain() {
    fn validate_and_process(short_key: &str) -> AppResult<u64> {
        validate_short_key(short_key)?;
        let (id, _) = split_short_key(short_key);
        if id == 0 {
            return Err(AppError::NotFound("URL not found".to_string()));
        }
        Ok(id)
    }

    // 유효한 short_key (4자 랜덤키 사용)
    let valid_key = merge_short_key("AbXy", 123);
    assert!(validate_and_process(&valid_key).is_ok());

    // 너무 짧은 short_key (5자 미만)
    assert!(matches!(
        validate_and_process("abcd"),
        Err(AppError::BadRequest(_))
    ));

    // 특수문자 포함
    assert!(matches!(
        validate_and_process("ab-cdef"),
        Err(AppError::BadRequest(_))
    ));
}

// ============ 데이터 구조 통합 테스트 ============

#[test]
fn test_url_to_cache_data_conversion() {
    use chrono::Utc;

    let url = Url {
        id: 1,
        random_key: "AbXy".to_string(),
        ios_deep_link: Some("app://ios".to_string()),
        ios_fallback_url: Some("https://apps.apple.com".to_string()),
        android_deep_link: Some("app://android".to_string()),
        android_fallback_url: Some("https://play.google.com".to_string()),
        default_fallback_url: "https://example.com".to_string(),
        hashed_value: "hash123".to_string(),
        webhook_url: Some("https://webhook.example.com".to_string()),
        og_title: Some("Title".to_string()),
        og_description: Some("Description".to_string()),
        og_image_url: Some("https://example.com/image.png".to_string()),
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };

    let cache_data: UrlCacheData = url.clone().into();

    // 필드 매핑 확인
    assert_eq!(cache_data.id, url.id);
    assert_eq!(cache_data.random_key, url.random_key);
    assert_eq!(cache_data.ios_deep_link, url.ios_deep_link);
    assert_eq!(cache_data.default_fallback_url, url.default_fallback_url);
    assert_eq!(cache_data.webhook_url, url.webhook_url);
    assert_eq!(cache_data.is_active, url.is_active);
}

#[test]
fn test_new_url_creation() {
    let new_url = NewUrl {
        random_key: gen_rand_str(4),
        ios_deep_link: Some("https://ios.example.com".to_string()),
        ios_fallback_url: None,
        android_deep_link: None,
        android_fallback_url: None,
        default_fallback_url: "https://example.com".to_string(),
        hashed_value: "hash123abc".to_string(),
        webhook_url: None,
        og_title: Some("Test".to_string()),
        og_description: None,
        og_image_url: None,
        is_active: true,
    };

    assert_eq!(new_url.random_key.len(), 4);
    assert!(new_url.is_active);
}

// ============ 직렬화 통합 테스트 ============

#[test]
fn test_request_response_serialization() {
    // Request 역직렬화 테스트 (CreateShortUrlRequest는 Deserialize만 구현)
    let req_json = r#"{"defaultFallbackUrl": "https://example.com"}"#;
    let req: CreateShortUrlRequest = serde_json::from_str(req_json).unwrap();
    assert_eq!(
        req.default_fallback_url,
        Some("https://example.com".to_string())
    );

    // Response 직렬화 테스트
    let resp = CreateShortUrlResponse::created("Ab3D7Xy".to_string());
    let resp_json = serde_json::to_string(&resp).unwrap();
    assert!(resp_json.contains("Ab3D7Xy"));
    assert!(resp_json.contains("URL created successfully"));
}

#[test]
fn test_url_cache_data_messagepack_serialization() {
    let cache_data = UrlCacheData {
        id: 1,
        random_key: "AbXy".to_string(),
        ios_deep_link: Some("app://ios".to_string()),
        ios_fallback_url: None,
        android_deep_link: None,
        android_fallback_url: None,
        default_fallback_url: "https://example.com".to_string(),
        webhook_url: None,
        og_title: Some("Title".to_string()),
        og_description: None,
        og_image_url: None,
        is_active: true,
    };

    // MessagePack 직렬화
    let packed = rmp_serde::to_vec(&cache_data).unwrap();
    assert!(!packed.is_empty());

    // MessagePack 역직렬화
    let unpacked: UrlCacheData = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(cache_data.id, unpacked.id);
    assert_eq!(cache_data.random_key, unpacked.random_key);
    assert_eq!(
        cache_data.default_fallback_url,
        unpacked.default_fallback_url
    );
}

// ============ 유효성 검사 통합 테스트 ============

#[test]
fn test_request_validation_scenarios() {
    // 유효한 요청
    let valid_req = CreateShortUrlRequest {
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
    assert!(valid_req.validate().is_ok());

    // 필수 필드 누락
    let missing_url = CreateShortUrlRequest {
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
    assert!(missing_url.validate().is_err());

    // 잘못된 URL 형식
    let invalid_url = CreateShortUrlRequest {
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
    assert!(invalid_url.validate().is_err());

    // OG 필드 길이 초과
    let long_title = CreateShortUrlRequest {
        ios_deep_link: None,
        ios_fallback_url: None,
        android_deep_link: None,
        android_fallback_url: None,
        default_fallback_url: Some("https://example.com".to_string()),
        webhook_url: None,
        og_title: Some("a".repeat(256)),
        og_description: None,
        og_image_url: None,
    };
    assert!(long_title.validate().is_err());
}

// ============ 랜덤 문자열 통합 테스트 ============

#[test]
fn test_random_string_uniqueness_over_many_generations() {
    use std::collections::HashSet;

    let mut generated: HashSet<String> = HashSet::new();

    for _ in 0..1000 {
        let rand_str = gen_rand_str(8);
        assert!(
            generated.insert(rand_str.clone()),
            "Duplicate random string generated: {rand_str}"
        );
    }

    assert_eq!(generated.len(), 1000);
}

// ============ 해시 생성 통합 테스트 ============

#[test]
fn test_hash_generation_for_duplicate_detection() {
    use xxhash_rust::xxh3::xxh3_128;

    let create_hash = |ios: &str, android: &str, default: &str| -> String {
        let hash_input = format!("{}:{}:{}:{}:{}", ios, "", android, "", default);
        format!("{:032x}", xxh3_128(hash_input.as_bytes()))
    };

    // 동일한 입력은 동일한 해시
    let hash1 = create_hash("app://ios", "app://android", "https://example.com");
    let hash2 = create_hash("app://ios", "app://android", "https://example.com");
    assert_eq!(hash1, hash2);

    // 다른 입력은 다른 해시
    let hash3 = create_hash("app://ios2", "app://android", "https://example.com");
    assert_ne!(hash1, hash3);

    // 해시 길이 확인
    assert_eq!(hash1.len(), 32);
}

// ============ JWT Claims 통합 테스트 ============

#[test]
fn test_jwt_claims_expiration() {
    let token = gen_token("test_user").expect("Failed to generate token");
    let claims = parse_token(&token).expect("Failed to parse token");

    let now = chrono::Utc::now().timestamp();

    // 토큰이 아직 유효한지 확인
    assert!(claims.exp > now);

    // 발급 시간이 현재 시간 이전인지 확인
    assert!(claims.iat <= now);

    // exp가 iat보다 큰지 확인
    assert!(claims.exp > claims.iat);
}

// ============ 에러 타입 통합 테스트 ============

#[test]
fn test_error_types_and_messages() {
    let bad_request = AppError::BadRequest("Invalid input".to_string());
    assert!(bad_request.to_string().contains("Invalid input"));

    let not_found = AppError::NotFound("Resource not found".to_string());
    assert!(not_found.to_string().contains("Resource not found"));

    let unauthorized = AppError::Unauthorized("Token expired".to_string());
    assert!(unauthorized.to_string().contains("Token expired"));

    let validation = AppError::Validation("Field is required".to_string());
    assert!(validation.to_string().contains("Field is required"));

    let internal = AppError::Internal("Server error".to_string());
    assert!(internal.to_string().contains("Server error"));
}

// ============ Short Key 경계값 테스트 ============

#[test]
fn test_short_key_boundary_values() {
    // 최소 유효 ID (4자 랜덤키 사용)
    let min_key = merge_short_key("AbXy", 1);
    assert!(validate_short_key(&min_key).is_ok());

    // 0 ID
    let zero_key = merge_short_key("AbXy", 0);
    assert!(validate_short_key(&zero_key).is_ok());

    // 매우 큰 ID
    let large_key = merge_short_key("AbXy", u64::MAX / 2);
    assert!(validate_short_key(&large_key).is_ok());

    // 최대 ID
    let max_key = merge_short_key("AbXy", u64::MAX);
    assert!(validate_short_key(&max_key).is_ok());
}

// ============ 전체 URL 생성 흐름 시뮬레이션 ============

#[test]
fn test_complete_url_creation_simulation() {
    use xxhash_rust::xxh3::xxh3_128;

    // 1. 요청 데이터
    let default_fallback = "https://example.com/landing";
    let ios_deep_link = "myapp://product/123";

    // 2. 해시 생성 (중복 방지용)
    let hash_input = format!(
        "{}:{}:{}:{}:{}",
        ios_deep_link, "", "", "", default_fallback
    );
    let hashed_value = format!("{:032x}", xxh3_128(hash_input.as_bytes()));
    assert_eq!(hashed_value.len(), 32);

    // 3. 랜덤 키 생성 (4자: prefix 2자 + suffix 2자)
    let random_key = gen_rand_str(4);
    assert_eq!(random_key.len(), 4);

    // 4. NewUrl 생성
    let new_url = NewUrl {
        random_key: random_key.clone(),
        ios_deep_link: Some(ios_deep_link.to_string()),
        ios_fallback_url: None,
        android_deep_link: None,
        android_fallback_url: None,
        default_fallback_url: default_fallback.to_string(),
        hashed_value,
        webhook_url: None,
        og_title: None,
        og_description: None,
        og_image_url: None,
        is_active: true,
    };

    // 5. ID 시뮬레이션 (DB에서 반환될 값)
    let simulated_id: u64 = 98765;

    // 6. Short key 생성 (prefix + base62(id) + suffix)
    let short_key = merge_short_key(&new_url.random_key, simulated_id);

    // 7. 유효성 검사
    assert!(validate_short_key(&short_key).is_ok());

    // 8. Short key 구조 확인
    let prefix = &random_key[..2];
    let suffix = &random_key[2..];
    assert!(short_key.starts_with(prefix));
    assert!(short_key.ends_with(suffix));

    // 9. 응답 생성
    let response = CreateShortUrlResponse::created(short_key.clone());
    assert_eq!(response.short_key, Some(short_key));
    assert!(response.message.contains("created"));
}

// ============ 새로운 Short Key 형식 테스트 ============

#[test]
fn test_new_short_key_format() {
    let rand_key = "PrSf"; // Prefix="Pr", Suffix="Sf"
    let id: u64 = 12345;

    let short_key = merge_short_key(rand_key, id);

    // 형식 확인: Pr + base62(12345) + Sf
    assert!(short_key.starts_with("Pr"));
    assert!(short_key.ends_with("Sf"));

    // 디코딩 확인
    let (decoded_id, decoded_rand_key) = split_short_key(&short_key);
    assert_eq!(decoded_id, id);
    assert_eq!(decoded_rand_key, rand_key);
}

#[test]
fn test_short_key_prefix_suffix_extraction() {
    for id in [0, 1, 62, 3844, 1_000_000, u64::MAX] {
        let rand_key = "AaZz";
        let short_key = merge_short_key(rand_key, id);

        // 앞 2자와 뒤 2자 확인
        assert!(short_key.starts_with("Aa"));
        assert!(short_key.ends_with("Zz"));

        // 라운드트립 확인
        let (decoded_id, decoded_rand) = split_short_key(&short_key);
        assert_eq!(decoded_id, id);
        assert_eq!(decoded_rand, rand_key);
    }
}

// ============ AppConfig 환경 변수 테스트 ============

#[test]
fn test_app_config_is_accessible() {
    use url_shortener::config::APP_CONFIG;

    // APP_CONFIG 전역 인스턴스에 접근 가능한지 확인
    assert!(!APP_CONFIG.server_port.is_empty());
    assert!(APP_CONFIG.db_max_connections > 0);
    assert!(APP_CONFIG.redis_max_connections > 0);
}

#[test]
fn test_app_config_production_mode_check() {
    use url_shortener::config::APP_CONFIG;

    // is_production 필드가 올바르게 설정되는지 확인
    // 테스트 환경에서는 RUST_ENV가 설정되지 않아 false일 것
    let is_prod_from_env = std::env::var("RUST_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);
    assert_eq!(APP_CONFIG.is_production, is_prod_from_env);
}

// ============ Health Check 응답 구조 테스트 ============

#[test]
fn test_health_response_structure() {
    use url_shortener::api::handlers::{HealthResponse, ReadinessResponse};

    let health = HealthResponse {
        status: "ok",
        version: "0.1.0",
    };

    let json = serde_json::to_string(&health).unwrap();
    assert!(json.contains("status"));
    assert!(json.contains("version"));

    let readiness = ReadinessResponse {
        status: "ok",
        database: "connected",
        cache: "connected",
    };

    let json = serde_json::to_string(&readiness).unwrap();
    assert!(json.contains("database"));
    assert!(json.contains("cache"));
}
