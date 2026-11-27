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
    auth::{authenticate, limit_writes, Auth},
    cache::LinkCache,
    config::Config,
    error::{json_error, ApiJson, AppError, AppResult},
    links::{self, LinkFields, LinkRecord},
    redirect::{PagePolicies, INDEX_HTML},
    short_key::{encode_short_key, parse_short_key},
    webhook::Webhooks,
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) auth: Arc<Auth>,
    db: sqlx::PgPool,
    cache: LinkCache,
    pages: Arc<PagePolicies>,
    webhooks: Webhooks,
    shutdown: CancellationToken,
}

impl AppState {
    pub async fn new(db: sqlx::PgPool, config: &Config) -> AppResult<Self> {
        Ok(Self {
            db,
            auth: Arc::new(Auth::new(
                config.jwt_secret.as_bytes(),
                config.jwt_ttl,
                config.production,
            )),
            pages: Arc::new(PagePolicies::new()?),
            webhooks: Webhooks::new()
                .map_err(|_| AppError::Internal("Unable to create webhook client"))?,
            cache: LinkCache::new(config.cache_max_capacity_bytes, config.redis.clone())
                .await
                .map_err(|_| AppError::Internal("Invalid Redis client configuration"))?,
            shutdown: CancellationToken::new(),
        })
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        tokio::join!(self.webhooks.drain(), self.db.close());
    }
}

pub fn create_routes(state: AppState, config: &Config) -> AppResult<Router> {
    let writes = Router::new()
        .route(
            "/v1/urls",
            post(create_link)
                .route_layer(middleware::from_fn_with_state(state.clone(), authenticate)),
        )
        .route("/", get(index));
    let writes = limit_writes(writes, config, state.shutdown.clone())?;
    let mut app = writes
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
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);
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
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

async fn index(State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    let cookie = HeaderValue::from_str(&state.auth.guest_cookie()?)
        .map_err(|_| AppError::Internal("Invalid Set-Cookie header"))?;
    Ok((
        [
            (header::SET_COOKIE, cookie),
            (header::CONTENT_SECURITY_POLICY, state.pages.index.clone()),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Html(INDEX_HTML),
    ))
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
    let (db, cache) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(2),
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.db)
        ),
        state.cache.status(),
    );
    let (code, status, database) = match (db, cache) {
        (Ok(Ok(1)), "disconnected") => (StatusCode::OK, "degraded", "connected"),
        (Ok(Ok(1)), _) => (StatusCode::OK, "ok", "connected"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "error", "disconnected"),
    };
    (
        code,
        Json(serde_json::json!({"status": status, "database": database, "cache": cache})),
    )
}
