//! Shared application state, Axum router assembly, and thin HTTP handlers.

use std::{sync::Arc, time::Duration};

use axum::{
    extract::rejection::PathRejection,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};

use crate::{
    cache::LinkCache,
    config::Config,
    error::{json_error, ApiJson, AppError, AppResult},
    guard::{limit_writes, verify_origin, OriginGuard},
    links::{self, LinkFields, LinkRecord},
    redirect::{PagePolicies, INDEX_HTML},
    short_key::{encode_short_key, parse_short_key},
    webhook::Webhooks,
};

/// Cloneable state shared across handlers (zero process-global mutable statics).
#[derive(Clone)]
pub struct AppState {
    pub(crate) origin: Arc<OriginGuard>,
    db: sqlx::PgPool,
    cache: LinkCache,
    pages: Arc<PagePolicies>,
    webhooks: Webhooks,
    shutdown: CancellationToken,
}

impl AppState {
    pub fn new(db: sqlx::PgPool, config: &Config) -> AppResult<Self> {
        Ok(Self {
            db,
            origin: Arc::new(OriginGuard::from_config(config)),
            pages: Arc::new(PagePolicies::new()?),
            webhooks: Webhooks::new()
                .map_err(|_| AppError::Internal("Unable to create webhook client"))?,
            cache: LinkCache::new(config.cache_max_capacity_bytes),
            shutdown: CancellationToken::new(),
        })
    }

    /// Cancels background tasks, drains in-flight webhooks (≤ 5 s), and closes Postgres.
    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        tokio::join!(self.webhooks.drain(), self.db.close());
    }
}

/// Builds the full HTTP router with write-scoped guards and global policy layers.
pub fn create_routes(state: AppState, config: &Config) -> AppResult<Router> {
    // Run `verify_origin` outside `limit_writes` so rejected cross-origin requests
    // return `403` without consuming the client's write rate-limit tokens.
    let writes = limit_writes(
        Router::new().route("/v1/urls", post(create_link)),
        config,
        state.shutdown.clone(),
    )?
    .route_layer(middleware::from_fn_with_state(state.clone(), verify_origin));

    let mut app = writes
        .route("/", get(index))
        .route("/health", get(health))
        .route("/ready", get(readiness))
        .route("/{short_key}", get(redirect))
        .fallback(|| async { AppError::not_found() })
        .method_not_allowed_fallback(|| async {
            json_error(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed")
        })
        .with_state(state);

    if !config.cors_origins.is_empty() {
        let cors = CorsLayer::new()
            .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
            .allow_headers([header::CONTENT_TYPE])
            .expose_headers([header::RETRY_AFTER]);
        app = app.layer(if config.cors_origins.iter().any(|origin| origin == "*") {
            cors.allow_origin(Any)
        } else {
            cors.allow_origin(config.cors_origins.clone())
        });
    }

    Ok(app
        .layer(DefaultBodyLimit::max(32 * 1024))
        .layer(middleware::from_fn(http_policy))
        .layer(TraceLayer::new_for_http()))
}

async fn http_policy(request: Request, next: Next) -> Response {
    let mut response = tokio::time::timeout(Duration::from_secs(15), next.run(request))
        .await
        .unwrap_or_else(|_| json_error(StatusCode::REQUEST_TIMEOUT, "Request timeout"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn index(State(state): State<AppState>) -> impl IntoResponse {
    (
        [
            (header::CONTENT_SECURITY_POLICY, state.pages.index.clone()),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        Html(INDEX_HTML),
    )
}

#[derive(Serialize)]
struct CreatedLink {
    message: &'static str,
    short_key: String,
    created: bool,
}

async fn create_link(
    State(state): State<AppState>,
    ApiJson(fields): ApiJson<LinkFields>,
) -> AppResult<(StatusCode, Json<CreatedLink>)> {
    let fields = fields.normalize()?;
    let (identity, created) = links::create_or_find(&state.db, &fields).await?;
    let short_key = encode_short_key(identity.id, &identity.random_key)
        .ok_or(AppError::Internal("Invalid stored short key"))?;
    let (status, message) = if created {
        state
            .cache
            .warm(LinkRecord {
                id: identity.id,
                random_key: identity.random_key,
                fields,
            })
            .await?;
        (StatusCode::CREATED, "URL shortened successfully")
    } else {
        (StatusCode::OK, "URL already exists")
    };
    Ok((
        status,
        Json(CreatedLink {
            message,
            short_key,
            created,
        }),
    ))
}

async fn redirect(
    State(state): State<AppState>,
    key: Result<Path<String>, PathRejection>,
    headers: axum::http::HeaderMap,
    method: Method,
) -> AppResult<Response> {
    let Path(key) = key.map_err(|_| AppError::not_found())?;
    let (id, random_key) = parse_short_key(&key).ok_or_else(AppError::not_found)?;
    let link = state
        .cache
        .get(&state.db, id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "Link lookup failed");
            sentry::capture_error(error.as_ref());
            AppError::Internal("Link lookup failed")
        })?
        .filter(|link| link.random_key == random_key)
        .ok_or_else(AppError::not_found)?;

    if let Some(url) = &link.webhook_url {
        if method == Method::GET {
            let user_agent = headers
                .get(header::USER_AGENT)
                .and_then(|agent| agent.to_str().ok())
                .unwrap_or("Unknown");
            state.webhooks.dispatch(url, key, user_agent);
        }
    }
    Ok(link.response(&state.pages))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    let db = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.db),
    )
    .await;
    if matches!(db, Ok(Ok(1))) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "database": "connected"})),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "error", "database": "disconnected"})),
        )
    }
}
