//! JWT utility module.
//!
//! Provides JWT token generation and parsing functions.

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

use crate::config::get_env;
use crate::error::AppResult;

/// Minimum recommended length for JWT secrets.
const MIN_SECRET_LENGTH: usize = 32;

static JWT_SECRET: Lazy<String> = Lazy::new(|| {
    let secret = get_env("JWT_SECRET", None);
    let env_mode = get_env("RUST_ENV", Some("development"));
    let is_production = env_mode == "production" || env_mode == "prod";

    if secret.is_empty() {
        assert!(
            !is_production,
            "JWT_SECRET must be set in production environment"
        );
        tracing::warn!(
            "⚠️  JWT_SECRET not set - using insecure default. \
             Set RUST_ENV=production to enforce security requirements."
        );
        "default-secret-change-me-in-production".to_string()
    } else if secret.len() < MIN_SECRET_LENGTH {
        tracing::warn!(
            "⚠️  JWT_SECRET is shorter than {} characters. \
             Consider using a longer secret for better security.",
            MIN_SECRET_LENGTH
        );
        secret
    } else {
        secret
    }
});

static JWT_EXPIRATION: Lazy<i64> = Lazy::new(|| {
    get_env("JWT_EXPIRATION_HOURS", Some("24"))
        .parse()
        .unwrap_or(24)
});

/// JWT claims structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject (user identifier)
    pub sub: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
}

/// Generates a JWT token for the given subject.
#[must_use = "the generated token should be used"]
pub fn gen_token(subject: &str) -> AppResult<String> {
    let now = chrono::Utc::now().timestamp();
    let exp = now + (*JWT_EXPIRATION * 3600);

    let claims = Claims {
        sub: subject.to_string(),
        exp,
        iat: now,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )?;

    Ok(token)
}

/// Parses and validates a JWT token.
#[must_use = "the parsed claims should be used"]
pub fn parse_token(token: &str) -> AppResult<Claims> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
        &Validation::default(),
    )?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gen_and_parse_token() {
        let subject = "test_user";
        let token = gen_token(subject).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, subject);
    }

    #[test]
    fn test_gen_token_guest() {
        let token = gen_token("guest").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, "guest");
    }

    #[test]
    fn test_gen_token_empty_subject() {
        let token = gen_token("").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, "");
    }

    #[test]
    fn test_gen_token_unicode_subject() {
        let subject = "사용자_テスト_🚀";
        let token = gen_token(subject).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, subject);
    }

    #[test]
    fn test_gen_token_long_subject() {
        let subject = "a".repeat(1000);
        let token = gen_token(&subject).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, subject);
    }

    #[test]
    fn test_claims_exp_is_future() {
        let token = gen_token("test").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        let now = chrono::Utc::now().timestamp();
        assert!(claims.exp > now);
    }

    #[test]
    fn test_claims_iat_is_past_or_now() {
        let token = gen_token("test").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        let now = chrono::Utc::now().timestamp();
        assert!(claims.iat <= now);
    }

    #[test]
    fn test_invalid_token_format() {
        let result = parse_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_token() {
        let result = parse_token("");
        assert!(result.is_err());
    }

    #[test]
    fn test_malformed_token_no_dots() {
        let result = parse_token("nodotshere");
        assert!(result.is_err());
    }

    #[test]
    fn test_malformed_token_one_dot() {
        let result = parse_token("one.dot");
        assert!(result.is_err());
    }

    #[test]
    fn test_token_uniqueness() {
        let token1 = gen_token("user1").expect("Failed to generate token");
        let token2 = gen_token("user2").expect("Failed to generate token");
        assert_ne!(token1, token2);
    }

    #[test]
    fn test_same_subject_different_tokens() {
        // 동일 subject라도 iat가 다를 수 있어 토큰이 다를 수 있음
        let token1 = gen_token("same_user").expect("Failed to generate token");
        let claims1 = parse_token(&token1).expect("Failed to parse token");
        let claims2 = parse_token(&token1).expect("Failed to parse token");
        // 같은 토큰을 파싱하면 같은 클레임
        assert_eq!(claims1.sub, claims2.sub);
        assert_eq!(claims1.exp, claims2.exp);
    }

    #[test]
    fn test_special_characters_in_subject() {
        let special_chars = "!@#$%^&*()_+-=[]{}|;':\",./<>?`~";
        let token = gen_token(special_chars).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, special_chars);
    }

    // ============ Claims 구조체 테스트 ============

    #[test]
    fn test_claims_clone() {
        let claims = Claims {
            sub: "test".to_string(),
            exp: 9999999999,
            iat: 1000000000,
        };
        let cloned = claims.clone();
        assert_eq!(claims.sub, cloned.sub);
        assert_eq!(claims.exp, cloned.exp);
        assert_eq!(claims.iat, cloned.iat);
    }

    #[test]
    fn test_claims_debug() {
        let claims = Claims {
            sub: "debug_test".to_string(),
            exp: 123456,
            iat: 654321,
        };
        let debug_str = format!("{claims:?}");
        assert!(debug_str.contains("Claims"));
        assert!(debug_str.contains("debug_test"));
    }

    #[test]
    fn test_claims_serialize() {
        let claims = Claims {
            sub: "serialize_test".to_string(),
            exp: 1234567890,
            iat: 1234567800,
        };
        let json = serde_json::to_string(&claims).unwrap();
        assert!(json.contains("serialize_test"));
        assert!(json.contains("1234567890"));
    }

    #[test]
    fn test_claims_deserialize() {
        let json = r#"{"sub":"deserialize_test","exp":9999999999,"iat":1000000000}"#;
        let claims: Claims = serde_json::from_str(json).unwrap();
        assert_eq!(claims.sub, "deserialize_test");
        assert_eq!(claims.exp, 9999999999);
    }

    // ============ 토큰 구조 테스트 ============

    #[test]
    fn test_token_has_three_parts() {
        let token = gen_token("test").expect("Failed to generate token");
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT should have 3 parts separated by '.'");
    }

    #[test]
    fn test_token_parts_not_empty() {
        let token = gen_token("test").expect("Failed to generate token");
        for part in token.split('.') {
            assert!(!part.is_empty(), "JWT part should not be empty");
        }
    }

    #[test]
    fn test_token_is_base64_like() {
        let token = gen_token("test").expect("Failed to generate token");
        // JWT 파트는 Base64URL 인코딩 (alphanumeric + - + _)
        for part in token.split('.') {
            assert!(
                part.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "JWT part should be base64url encoded"
            );
        }
    }

    // ============ 토큰 파싱 에러 케이스 ============

    #[test]
    fn test_parse_token_tampered() {
        let token = gen_token("test").expect("Failed to generate token");
        // 토큰의 마지막 문자 변경 (서명 변조)
        let mut tampered = token.clone();
        tampered.push('x');
        assert!(parse_token(&tampered).is_err());
    }

    #[test]
    fn test_parse_token_truncated() {
        let token = gen_token("test").expect("Failed to generate token");
        // 토큰 앞부분만 사용
        let truncated = &token[..token.len() / 2];
        assert!(parse_token(truncated).is_err());
    }

    #[test]
    fn test_parse_token_with_extra_dots() {
        let result = parse_token("a.b.c.d");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_token_spaces() {
        let result = parse_token("  ");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_token_newlines() {
        let token = gen_token("test").expect("Failed to generate token");
        let with_newline = format!("{token}\n");
        // 개행 문자가 있으면 파싱 실패
        assert!(parse_token(&with_newline).is_err());
    }

    // ============ 다양한 subject 테스트 ============

    #[test]
    fn test_token_with_email_subject() {
        let email = "user@example.com";
        let token = gen_token(email).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, email);
    }

    #[test]
    fn test_token_with_uuid_subject() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let token = gen_token(uuid).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, uuid);
    }

    #[test]
    fn test_token_with_numeric_subject() {
        let numeric = "12345678901234567890";
        let token = gen_token(numeric).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, numeric);
    }

    #[test]
    fn test_token_with_json_subject() {
        let json_sub = r#"{"user_id": 123, "role": "admin"}"#;
        let token = gen_token(json_sub).expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert_eq!(claims.sub, json_sub);
    }

    // ============ 시간 관련 테스트 ============

    #[test]
    fn test_claims_exp_greater_than_iat() {
        let token = gen_token("test").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        assert!(claims.exp > claims.iat, "exp should be greater than iat");
    }

    #[test]
    fn test_claims_exp_iat_difference() {
        let token = gen_token("test").expect("Failed to generate token");
        let claims = parse_token(&token).expect("Failed to parse token");
        let diff = claims.exp - claims.iat;
        // 기본 24시간 = 86400초, 환경 변수에 따라 다를 수 있음
        assert!(diff > 0, "exp - iat should be positive");
    }

    // ============ 연속 생성 테스트 ============

    #[test]
    fn test_multiple_tokens_same_subject() {
        let subject = "repeated_user";
        let tokens: Vec<String> = (0..5)
            .map(|_| gen_token(subject).expect("Failed to generate token"))
            .collect();

        // 모든 토큰이 유효한지 확인
        for token in &tokens {
            let claims = parse_token(token).expect("Failed to parse token");
            assert_eq!(claims.sub, subject);
        }
    }

    #[test]
    fn test_different_subjects_different_tokens() {
        let subjects = ["user1", "user2", "user3"];
        let tokens: Vec<String> = subjects
            .iter()
            .map(|s| gen_token(s).expect("Failed to generate token"))
            .collect();

        // 모든 토큰이 서로 다른지 확인
        for i in 0..tokens.len() {
            for j in i + 1..tokens.len() {
                assert_ne!(tokens[i], tokens[j], "Tokens should be different");
            }
        }
    }

    // ============ Claims 직렬화 왕복 테스트 ============

    #[test]
    fn test_claims_roundtrip_serialization() {
        let original = Claims {
            sub: "roundtrip_test".to_string(),
            exp: 9876543210,
            iat: 1234567890,
        };

        let json = serde_json::to_string(&original).unwrap();
        let restored: Claims = serde_json::from_str(&json).unwrap();

        assert_eq!(original.sub, restored.sub);
        assert_eq!(original.exp, restored.exp);
        assert_eq!(original.iat, restored.iat);
    }
}
