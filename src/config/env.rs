//! Environment variable configuration module.
//!
//! Provides environment variable loading and the global `APP_CONFIG` instance.

use std::env;
use std::sync::Once;

use once_cell::sync::Lazy;

static INIT: Once = Once::new();

/// Initializes the environment by loading the .env file.
/// This is called automatically when `get_env` is first used.
fn init_env() {
    INIT.call_once(|| {
        if let Err(e) = dotenvy::dotenv() {
            tracing::warn!("Warning: .env file not found or error loading: {}", e);
        }
    });
}

/// Retrieves an environment variable by key.
///
/// If the variable is not set, returns the provided default value.
/// If no default is provided and the variable is not set, returns an empty string.
pub fn get_env(key: &str, default: Option<&str>) -> String {
    init_env();
    env::var(key).unwrap_or_else(|_| default.unwrap_or("").to_string())
}

/// Retrieves an environment variable as a parsed type.
pub fn get_env_parsed<T: std::str::FromStr>(key: &str, default: T) -> T {
    init_env();
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Application configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct AppConfig {
    // Server settings
    pub server_port: String,

    // Environment
    pub is_production: bool,

    // Sentry settings
    pub sentry_dsn: String,
    pub sentry_traces_sample_rate: f32,

    // Database settings
    pub db_max_connections: u32,
    pub db_min_connections: u32,
    pub db_acquire_timeout_secs: u64,
    pub db_idle_timeout_secs: u64,
    pub db_max_lifetime_secs: u64,

    // Cache settings
    pub cache_ttl_secs: u64,
    pub redis_max_connections: usize,

    // CORS settings
    pub cors_origins: String,

    // Rate limiting
    pub rate_limit_per_second: u64,
    pub rate_limit_burst_size: u32,

    // Webhook settings
    pub webhook_timeout_secs: u64,
    pub webhook_max_concurrent: usize,

    // Migration
    pub run_migrations: bool,
}

impl AppConfig {
    /// Creates a new `AppConfig` from environment variables.
    pub fn from_env() -> Self {
        let rust_env = get_env("RUST_ENV", Some("development"));
        let is_production = rust_env == "production" || rust_env == "prod";

        Self {
            server_port: get_env("SERVER_PORT", Some("3000")),

            is_production,

            sentry_dsn: get_env("SENTRY_DSN", None),
            sentry_traces_sample_rate: get_env_parsed("SENTRY_TRACES_SAMPLE_RATE", 0.1),

            db_max_connections: get_env_parsed("DB_MAX_CONNECTIONS", 20),
            db_min_connections: get_env_parsed("DB_MIN_CONNECTIONS", 2),
            db_acquire_timeout_secs: get_env_parsed("DB_ACQUIRE_TIMEOUT_SECS", 5),
            db_idle_timeout_secs: get_env_parsed("DB_IDLE_TIMEOUT_SECS", 600),
            db_max_lifetime_secs: get_env_parsed("DB_MAX_LIFETIME_SECS", 1800),

            cache_ttl_secs: get_env_parsed("CACHE_TTL_SECS", 3600),
            redis_max_connections: get_env_parsed("REDIS_MAX_CONNECTIONS", 20),

            cors_origins: get_env("CORS_ORIGINS", Some("*")),

            rate_limit_per_second: get_env_parsed("RATE_LIMIT_PER_SECOND", 10),
            rate_limit_burst_size: get_env_parsed("RATE_LIMIT_BURST_SIZE", 50),

            webhook_timeout_secs: get_env_parsed("WEBHOOK_TIMEOUT_SECS", 10),
            webhook_max_concurrent: get_env_parsed("WEBHOOK_MAX_CONCURRENT", 100),

            run_migrations: get_env("RUN_MIGRATIONS", Some("true")) == "true",
        }
    }
}

/// Global application configuration instance.
pub static APP_CONFIG: Lazy<AppConfig> = Lazy::new(AppConfig::from_env);

#[cfg(test)]
mod tests {
    use super::*;

    // ============ get_env 함수 테스트 ============

    #[test]
    fn test_get_env_with_default() {
        // 존재하지 않는 환경 변수는 기본값을 반환해야 함
        let result = get_env("NON_EXISTENT_VAR_FOR_TEST_12345", Some("default_value"));
        assert_eq!(result, "default_value");
    }

    #[test]
    fn test_get_env_no_default() {
        // 기본값 없이 존재하지 않는 환경 변수는 빈 문자열을 반환
        let result = get_env("NON_EXISTENT_VAR_FOR_TEST_67890", None);
        assert_eq!(result, "");
    }

    #[test]
    fn test_get_env_empty_default() {
        let result = get_env("NON_EXISTENT_VAR_99999", Some(""));
        assert_eq!(result, "");
    }

    // ============ get_env_parsed 함수 테스트 ============

    #[test]
    fn test_get_env_parsed_default_u32() {
        let result: u32 = get_env_parsed("NON_EXISTENT_U32_VAR", 42);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_get_env_parsed_default_u64() {
        let result: u64 = get_env_parsed("NON_EXISTENT_U64_VAR", 1000);
        assert_eq!(result, 1000);
    }

    #[test]
    fn test_get_env_parsed_default_f32() {
        let result: f32 = get_env_parsed("NON_EXISTENT_F32_VAR", 0.5);
        assert!((result - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_get_env_parsed_default_bool() {
        let result: bool = get_env_parsed("NON_EXISTENT_BOOL_VAR", true);
        assert!(result);
    }

    #[test]
    fn test_get_env_parsed_default_usize() {
        let result: usize = get_env_parsed("NON_EXISTENT_USIZE_VAR", 100);
        assert_eq!(result, 100);
    }

    // ============ AppConfig 구조체 테스트 ============

    #[test]
    fn test_app_config_from_env() {
        // 환경 변수가 설정되지 않은 경우 기본값 사용
        let config = AppConfig::from_env();

        // 기본값 확인
        assert!(!config.server_port.is_empty());
        assert!(config.db_max_connections > 0);
        assert!(config.db_min_connections > 0);
        assert!(config.cache_ttl_secs > 0);
        assert!(config.rate_limit_per_second > 0);
        assert!(config.rate_limit_burst_size > 0);
    }

    #[test]
    fn test_app_config_default_values() {
        let config = AppConfig::from_env();

        // 기본값들이 예상한 범위 내인지 확인
        assert_eq!(config.server_port, get_env("SERVER_PORT", Some("3000")));
        assert!(config.db_max_connections >= config.db_min_connections);
    }

    #[test]
    fn test_app_config_clone() {
        let config = AppConfig::from_env();
        let cloned = config.clone();

        assert_eq!(config.server_port, cloned.server_port);
        assert_eq!(config.db_max_connections, cloned.db_max_connections);
        assert_eq!(config.cache_ttl_secs, cloned.cache_ttl_secs);
    }

    #[test]
    fn test_app_config_debug() {
        let config = AppConfig::from_env();
        let debug_str = format!("{config:?}");

        assert!(debug_str.contains("AppConfig"));
        assert!(debug_str.contains("server_port"));
        assert!(debug_str.contains("db_max_connections"));
    }

    #[test]
    fn test_app_config_sentry_traces_sample_rate_range() {
        let config = AppConfig::from_env();
        // sample rate는 0.0 ~ 1.0 범위여야 함
        assert!(config.sentry_traces_sample_rate >= 0.0);
        assert!(config.sentry_traces_sample_rate <= 1.0);
    }

    #[test]
    fn test_app_config_timeout_values_positive() {
        let config = AppConfig::from_env();
        assert!(config.db_acquire_timeout_secs > 0);
        assert!(config.db_idle_timeout_secs > 0);
        assert!(config.db_max_lifetime_secs > 0);
        assert!(config.webhook_timeout_secs > 0);
    }

    #[test]
    fn test_app_config_webhook_max_concurrent_positive() {
        let config = AppConfig::from_env();
        assert!(config.webhook_max_concurrent > 0);
    }

    // ============ APP_CONFIG 전역 인스턴스 테스트 ============

    #[test]
    fn test_app_config_global_instance() {
        // APP_CONFIG에 접근 가능한지 확인
        let port = &APP_CONFIG.server_port;
        assert!(!port.is_empty());
    }

    #[test]
    fn test_app_config_global_same_instance() {
        // 여러 번 접근해도 같은 값을 반환하는지 확인
        let port1 = APP_CONFIG.server_port.clone();
        let port2 = APP_CONFIG.server_port.clone();
        assert_eq!(port1, port2);
    }

    // ============ 엣지 케이스 테스트 ============

    #[test]
    fn test_get_env_special_characters_in_default() {
        let result = get_env("NON_EXISTENT_SPECIAL", Some("!@#$%^&*()"));
        assert_eq!(result, "!@#$%^&*()");
    }

    #[test]
    fn test_get_env_unicode_default() {
        let result = get_env("NON_EXISTENT_UNICODE", Some("한글테스트"));
        assert_eq!(result, "한글테스트");
    }

    #[test]
    fn test_get_env_whitespace_default() {
        let result = get_env("NON_EXISTENT_WHITESPACE", Some("  spaces  "));
        assert_eq!(result, "  spaces  ");
    }

    // ============ 새로 추가된 필드 테스트 ============

    #[test]
    fn test_app_config_is_production_default() {
        // 기본적으로 development 모드
        let config = AppConfig::from_env();
        // RUST_ENV가 설정되지 않으면 development
        // 테스트 환경에서는 production이 아닐 것
        assert!(!config.is_production || std::env::var("RUST_ENV").is_ok());
    }

    #[test]
    fn test_app_config_redis_max_connections() {
        let config = AppConfig::from_env();
        assert!(config.redis_max_connections > 0);
    }

    #[test]
    fn test_app_config_redis_max_connections_default() {
        // 환경 변수가 설정되지 않았을 때 기본값 20
        let result: usize = get_env_parsed("REDIS_MAX_CONNECTIONS_NON_EXISTENT", 20);
        assert_eq!(result, 20);
    }

    #[test]
    fn test_app_config_has_is_production_field() {
        let config = AppConfig::from_env();
        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("is_production"));
    }

    #[test]
    fn test_app_config_has_redis_max_connections_field() {
        let config = AppConfig::from_env();
        let debug_str = format!("{config:?}");
        assert!(debug_str.contains("redis_max_connections"));
    }
}
