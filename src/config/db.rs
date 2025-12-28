//! 데이터베이스 설정 모듈.

use crate::config::env::{get_env, APP_CONFIG};
use crate::error::AppResult;
use once_cell::sync::OnceCell;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

static DB_POOL: OnceCell<PgPool> = OnceCell::new();

/// Initializes the database connection pool.
///
/// This function creates a connection pool and stores it in a global `OnceCell`.
/// Subsequent calls will return the same pool.
pub async fn init_db() -> AppResult<PgPool> {
    if let Some(pool) = DB_POOL.get() {
        return Ok(pool.clone());
    }

    let host = get_env("DB_HOST", Some("localhost"));
    let port = get_env("DB_PORT", Some("5432"));
    let user = get_env("DB_USER", Some("postgres"));
    let password = get_env("DB_PASSWORD", Some("postgres"));
    let dbname = get_env("DB_NAME", Some("postgres"));

    let database_url = format!("postgres://{user}:{password}@{host}:{port}/{dbname}");

    let pool = PgPoolOptions::new()
        .max_connections(APP_CONFIG.db_max_connections)
        .min_connections(APP_CONFIG.db_min_connections)
        .acquire_timeout(Duration::from_secs(APP_CONFIG.db_acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(APP_CONFIG.db_idle_timeout_secs))
        .max_lifetime(Duration::from_secs(APP_CONFIG.db_max_lifetime_secs))
        // Disable connection validation for performance (pool handles reconnection)
        .test_before_acquire(false)
        // Log slow connection acquisitions
        .acquire_slow_threshold(Duration::from_millis(500))
        .connect(&database_url)
        .await?;

    DB_POOL.set(pool.clone()).ok();
    tracing::info!(
        max_connections = APP_CONFIG.db_max_connections,
        min_connections = APP_CONFIG.db_min_connections,
        "Database connection pool established"
    );

    Ok(pool)
}

/// Closes the database connection pool.
pub async fn close_db() {
    if let Some(pool) = DB_POOL.get() {
        pool.close().await;
        tracing::info!("Database connection closed");
    }
}
