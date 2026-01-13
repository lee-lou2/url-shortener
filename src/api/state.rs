//! Application state module.
//!
//! Contains shared state for database and cache connections.

use deadpool_redis::Pool as RedisPool;
use sqlx::PgPool;

/// Shared application state.
///
/// This struct holds references to shared resources like database
/// and cache connections that handlers need access to.
#[derive(Clone)]
pub struct AppState {
    /// `PostgreSQL` connection pool
    pub db: PgPool,
    /// Redis connection pool
    pub cache: RedisPool,
}

impl AppState {
    /// Creates a new `AppState` instance.
    #[must_use]
    pub const fn new(db: PgPool, cache: RedisPool) -> Self {
        Self { db, cache }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // AppState는 실제 DB/Redis 연결이 필요하므로
    // 구조체 자체의 속성만 테스트

    #[test]
    fn test_app_state_is_clone() {
        // AppState가 Clone 트레이트를 구현하는지 컴파일 타임 확인
        fn assert_clone<T: Clone>() {}
        assert_clone::<AppState>();
    }

    #[test]
    fn test_app_state_struct_size() {
        // AppState 구조체 크기가 예상 범위 내인지 확인
        let size = std::mem::size_of::<AppState>();
        // PgPool과 RedisPool은 Arc 기반이므로 크기가 작아야 함
        assert!(size > 0);
        assert!(size < 256); // 합리적인 상한
    }
}
