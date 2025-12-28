//! Redis 캐시 설정 모듈.

use crate::config::env::{get_env, APP_CONFIG};
use crate::error::AppResult;
use deadpool_redis::{Config, Pool, PoolConfig, Runtime};
use once_cell::sync::OnceCell;

static CACHE_POOL: OnceCell<Pool> = OnceCell::new();

/// Initializes the Redis connection pool.
///
/// This function creates a connection pool and stores it in a global `OnceCell`.
/// Subsequent calls will return a clone of the same pool.
///
/// # Returns
///
/// A cloned Redis connection pool
///
/// # Errors
///
/// Returns an error if the Redis connection cannot be established
pub async fn init_cache() -> AppResult<Pool> {
    if let Some(pool) = CACHE_POOL.get() {
        return Ok(pool.clone());
    }

    let host = get_env("REDIS_HOST", Some("localhost"));
    let port = get_env("REDIS_PORT", Some("6379"));
    let password = get_env("REDIS_PASSWORD", None);

    let redis_url = if password.is_empty() {
        format!("redis://{host}:{port}")
    } else {
        format!("redis://:{password}@{host}:{port}")
    };

    let mut cfg = Config::from_url(redis_url);
    cfg.pool = Some(PoolConfig {
        max_size: APP_CONFIG.redis_max_connections,
        ..PoolConfig::default()
    });

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1))
        .map_err(|e| crate::error::AppError::Internal(format!("Redis pool error: {e}")))?;

    // Test connection
    let conn = pool.get().await.map_err(|e| {
        crate::error::AppError::Internal(format!("Redis connection test failed: {e}"))
    })?;
    drop(conn);

    CACHE_POOL.set(pool.clone()).ok();
    tracing::info!(
        max_connections = APP_CONFIG.redis_max_connections,
        "Redis connection pool established"
    );

    Ok(pool)
}

/// Closes the Redis connection pool.
///
/// Note: The pool handles cleanup automatically when dropped.
pub fn close_cache() {
    tracing::info!("Redis connection pool closed");
}
