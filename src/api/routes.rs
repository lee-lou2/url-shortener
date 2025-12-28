//! 라우트 설정 모듈.

use axum::{
    middleware,
    routing::{get, post},
    Router,
};

use crate::api::handlers::{
    create_short_url_handler, health_handler, index_handler, readiness_handler,
    redirect_to_original_handler,
};
use crate::api::middlewares::jwt_auth;
use crate::api::state::AppState;

/// Creates and configures all application routes.
///
/// # Routes
///
/// ## Health Check Routes
/// - `GET /health` - Liveness probe
/// - `GET /ready` - Readiness probe
///
/// ## Template Routes
/// - `GET /` - Main page
/// - `GET /:short_key` - Redirect to original URL
///
/// ## API Routes (v1)
/// - `POST /v1/urls` - Create short URL (requires JWT authentication)
pub fn create_routes(state: AppState) -> Router {
    // API v1 routes with JWT authentication
    let v1_routes = Router::new()
        .route("/urls", post(create_short_url_handler))
        .route_layer(middleware::from_fn(jwt_auth));

    // Main router
    Router::new()
        // Health check routes (no auth required)
        .route("/health", get(health_handler))
        .route("/ready", get(readiness_handler))
        // Template routes
        .route("/", get(index_handler))
        .route("/{short_key}", get(redirect_to_original_handler))
        // API routes
        .nest("/v1", v1_routes)
        // Shared state
        .with_state(state)
}
