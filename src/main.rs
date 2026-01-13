//! URL shortening service entry point.

mod api;
mod config;
mod error;
mod models;
mod utils;

use std::net::SocketAddr;
use std::time::Duration;

use axum::http::{header::HeaderValue, Method};
use tokio::signal;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::api::{create_routes, AppState};
use crate::config::{close_cache, close_db, init_cache, init_db, APP_CONFIG};

// High-performance memory allocator for non-MSVC targets
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Entry point for the URL shortening service.
#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "url_shortener=debug,tower_http=debug,axum=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Initialize Sentry
    let _guard = if APP_CONFIG.sentry_dsn.is_empty() {
        tracing::warn!("Sentry DSN not configured, error tracking disabled");
        None
    } else {
        Some(sentry::init((
            APP_CONFIG.sentry_dsn.clone(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: APP_CONFIG.sentry_traces_sample_rate,
                sample_rate: 1.0, // Capture all errors
                ..Default::default()
            },
        )))
    };

    // Initialize database
    let db = match init_db().await {
        Ok(pool) => pool,
        Err(e) => {
            tracing::error!("Failed to connect to database: {}", e);
            std::process::exit(1);
        }
    };

    // Run migrations if enabled
    if APP_CONFIG.run_migrations {
        tracing::info!("Running database migrations...");
        if let Err(e) = sqlx::migrate!("./migrations").run(&db).await {
            tracing::error!("Failed to run migrations: {}", e);
            std::process::exit(1);
        }
        tracing::info!("Database migrations completed");
    }

    // Initialize Redis cache
    let cache = match init_cache().await {
        Ok(manager) => manager,
        Err(e) => {
            tracing::error!("Failed to connect to Redis: {}", e);
            std::process::exit(1);
        }
    };

    // Create application state
    let state = AppState::new(db, cache);

    // Configure CORS based on environment
    let cors = build_cors_layer();

    // Configure rate limiting with SmartIpKeyExtractor for better IP detection
    let governor_config = GovernorConfigBuilder::default()
        .per_second(APP_CONFIG.rate_limit_per_second)
        .burst_size(APP_CONFIG.rate_limit_burst_size)
        .key_extractor(SmartIpKeyExtractor)
        .finish()
        .expect("Failed to build rate limiter config");

    // Create router with middleware
    // Layer order (bottom to top execution): CORS -> Compression -> Trace -> Rate Limit
    let app = create_routes(state)
        .layer(cors)
        .layer(
            CompressionLayer::new()
                .br(true)
                .gzip(true)
                .zstd(true)
                .quality(tower_http::compression::CompressionLevel::Default),
        )
        .layer(TraceLayer::new_for_http())
        .layer(GovernorLayer::new(governor_config));

    // Determine server address
    let port: u16 = APP_CONFIG.server_port.parse().unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!(
        port = port,
        rate_limit_per_second = APP_CONFIG.rate_limit_per_second,
        rate_limit_burst = APP_CONFIG.rate_limit_burst_size,
        "Starting server"
    );

    // Create TCP listener
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind to address");

    // Run server with graceful shutdown and ConnectInfo for rate limiting
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("Failed to start server");

    // Cleanup
    tracing::info!("Shutting down...");

    close_db().await;
    close_cache();

    // Flush Sentry events before exit
    if let Some(client) = sentry::Hub::current().client() {
        client.flush(Some(Duration::from_secs(2)));
    }

    tracing::info!("Shutdown complete");
}

/// Builds the CORS layer based on configuration.
fn build_cors_layer() -> CorsLayer {
    let cors_origins = &APP_CONFIG.cors_origins;

    if cors_origins == "*" {
        tracing::warn!("CORS is configured to allow all origins - not recommended for production");
        CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_origin(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
    } else {
        let origins: Vec<HeaderValue> = cors_origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();

        if origins.is_empty() {
            tracing::warn!("No valid CORS origins configured, allowing all");
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_origin(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any)
        } else {
            tracing::info!(origins = ?origins, "CORS configured with specific origins");
            CorsLayer::new()
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_origin(origins)
                .allow_headers(tower_http::cors::Any)
        }
    }
}

/// Handles shutdown signals for graceful termination.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {
            tracing::info!("Received Ctrl+C, initiating shutdown");
        },
        () = terminate => {
            tracing::info!("Received SIGTERM, initiating shutdown");
        },
    }
}
