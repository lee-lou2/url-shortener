use std::{
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::{header, Request, StatusCode},
    response::Response,
    Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgConnectOptions, PgPool};
use tower::ServiceExt;
use url_shortener::{
    config::Config,
    handlers::{create_routes, AppState},
    links::LinkFields,
    short_key::encode_short_key,
};

static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

struct TestApp {
    router: Router,
    state: AppState,
    db: PgPool,
}

fn destination() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let nonce: u64 = rand::random();
    format!("https://example.com/test/{time}-{nonce}")
}

async fn app_with(burst: &str, cors: Option<&str>) -> TestApp {
    let mut config = Config::from_lookup(|key| match key {
        "RUST_ENV" => Some("production".into()),
        "RATE_LIMIT_BURST_SIZE" => Some(burst.into()),
        "RATE_LIMIT_PER_SECOND" => Some("1".into()),
        "CORS_ORIGINS" => cors.map(str::to_owned),
        _ => None,
    })
    .unwrap();

    let db = if let Ok(url) =
        std::env::var("TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL"))
    {
        config.postgres = url.parse().unwrap();
        assert!(
            matches!(
                config.postgres.get_database(),
                Some("url_shortener_test" | "url_shortener")
            ),
            "Refusing to run integration tests against unexpected database: {:?}",
            config.postgres.get_database()
        );
        config.connect_db().await.unwrap()
    } else {
        let compose = PgConnectOptions::new()
            .host("127.0.0.1")
            .username("postgres")
            .password("postgres")
            .database("url_shortener");
        config.postgres = compose;
        if let Ok(pool) = config.connect_db().await {
            pool
        } else {
            config.postgres = PgConnectOptions::new()
                .host("127.0.0.1")
                .database("url_shortener_test");
            config.connect_db().await.unwrap()
        }
    };

    MIGRATED
        .get_or_init(|| async { sqlx::migrate!().run(&db).await.unwrap() })
        .await;
    let state = AppState::new(db.clone(), &config).unwrap();
    let router = create_routes(state.clone(), &config).unwrap();
    TestApp { router, state, db }
}

async fn app(burst: &str) -> TestApp {
    app_with(burst, None).await
}

impl TestApp {
    async fn get(&self, path: &str) -> Response {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        self.router.clone().oneshot(request).await.unwrap()
    }

    async fn post(&self, body: String, extra_headers: &[(&str, &str)]) -> Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/urls")
            .header(header::CONTENT_TYPE, "application/json");
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn create(&self, value: Value) -> Response {
        self.post(value.to_string(), &[]).await
    }

    async fn insert_legacy(&self, url: &str, hash: &str, active: bool) -> String {
        let id = sqlx::query_scalar(
            "INSERT INTO urls (random_key, default_fallback_url, hashed_value, is_active)
            VALUES ('AbXy', $1, $2, $3) RETURNING id",
        )
        .bind(url)
        .bind(hash)
        .bind(active)
        .fetch_one(&self.db)
        .await
        .unwrap();
        encode_short_key(id, "AbXy").unwrap()
    }

    async fn finish(self) {
        self.state.shutdown().await;
    }
}

async fn text(response: Response) -> String {
    let data = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    String::from_utf8(data.to_vec()).unwrap()
}

async fn body(response: Response) -> Value {
    serde_json::from_str(&text(response).await).unwrap()
}

async fn key(response: Response) -> String {
    body(response).await["short_key"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn check_csp(headers: &axum::http::HeaderMap, html: &str) {
    let script = html
        .rsplit_once("<script>")
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let digest = STANDARD.encode(Sha256::digest(script));
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains(&format!("'sha256-{digest}'")));
    assert!(csp.contains("frame-ancestors 'none'"));
}

#[tokio::test]
async fn index_sets_csp_without_cookies() {
    let app = app("20").await;
    let response = app.get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(headers.get(header::SET_COOKIE).is_none());
    check_csp(&headers, &text(response).await);
    app.finish().await;
}

#[tokio::test]
async fn cross_origin_guard_blocks_evil_sites_and_allows_same_origin_and_api() {
    let app = app_with("20", Some("https://trusted.example")).await;
    let payload = json!({"defaultFallbackUrl": destination()}).to_string();

    // Direct API client (no Fetch Metadata, no Origin) succeeds in 1 request.
    assert_eq!(
        app.post(payload.clone(), &[]).await.status(),
        StatusCode::CREATED
    );

    // Same-origin browser request succeeds.
    assert_eq!(
        app.post(payload.clone(), &[("sec-fetch-site", "same-origin")])
            .await
            .status(),
        StatusCode::OK
    );

    // Untrusted cross-site browser request is rejected with 403 JSON.
    let blocked = app
        .post(
            payload.clone(),
            &[
                ("sec-fetch-site", "cross-site"),
                ("origin", "https://evil.example"),
            ],
        )
        .await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body(blocked).await,
        json!({"error": "Cross-origin request rejected"})
    );

    // Explicitly allow-listed cross-site origin succeeds.
    let allowed = app
        .post(
            payload,
            &[
                ("sec-fetch-site", "cross-site"),
                ("origin", "https://trusted.example"),
            ],
        )
        .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    app.finish().await;
}

#[tokio::test]
async fn invalid_urls_and_json_are_rejected() {
    let app = app("20").await;
    for (name, value) in [
        ("defaultFallbackUrl", "javascript:0"),
        ("iosDeepLink", "data:text/plain,blocked"),
        ("webhookUrl", "http://127.0.0.1/"),
        ("webhookUrl", "http://169.254.169.254/"),
        ("androidFallbackUrl", "https://example.com/\r\n"),
    ] {
        let mut payload = json!({"defaultFallbackUrl": "https://example.com/"});
        payload[name] = json!(value);
        let response = app.create(payload).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{name}");
        assert!(body(response).await["error"]
            .as_str()
            .unwrap()
            .contains(name));
    }
    for (payload, status) in [
        ("{".into(), StatusCode::BAD_REQUEST),
        (
            r#"{"defaultFallbackUrl":42}"#.into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        ("x".repeat(33 * 1024), StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let response = app.post(payload, &[]).await;
        assert_eq!(response.status(), status);
        assert!(body(response).await["error"].is_string());
    }
    app.finish().await;
}

#[tokio::test]
async fn exact_dedupe_and_distinct_metadata() {
    let app = app("20").await;
    let url = destination();
    let created = app.create(json!({"defaultFallbackUrl": url})).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body(created).await;
    assert_eq!(
        created,
        json!({"created": true, "message": "URL shortened successfully", "short_key": created["short_key"]})
    );
    let duplicate = app
        .create(json!({"defaultFallbackUrl": url, "iosDeepLink": "  "}))
        .await;
    assert_eq!(duplicate.status(), StatusCode::OK);
    assert_eq!(
        body(duplicate).await,
        json!({"created": false, "message": "URL already exists", "short_key": created["short_key"]})
    );
    for payload in [
        json!({"defaultFallbackUrl": url, "ogTitle": "New title"}),
        json!({"defaultFallbackUrl": url, "webhookUrl": "https://example.net/hook"}),
    ] {
        let response = app.create(payload).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_ne!(key(response).await, created["short_key"].as_str().unwrap());
    }
    app.finish().await;
}

#[tokio::test]
async fn plain_redirect_uses_location_header() {
    let app = app("20").await;
    let url = destination();
    let key = key(app.create(json!({"defaultFallbackUrl": url})).await).await;
    let response = app.get(&format!("/{key}")).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()[header::LOCATION], url);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-cache"
    );
    assert_eq!(text(response).await, "");
    app.finish().await;
}

#[tokio::test]
async fn rich_redirect_html_escapes_attributes_and_hashes_its_script() {
    let app = app("20").await;
    let key = key(app
        .create(json!({
            "defaultFallbackUrl": destination(),
            "iosDeepLink": "myapp://open",
            "ogTitle": r#""><em>unsafe</em>"#
        }))
        .await)
    .await;
    let response = app.get(&format!("/{key}")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
    let html = text(response).await;
    assert!(
        html.contains("myapp://open")
            && html.contains("og:title")
            && !html.contains("<em>unsafe</em>")
    );
    check_csp(&headers, &html);
    app.finish().await;
}

#[tokio::test]
async fn existing_keys_resolve_and_unsafe_legacy_rows_are_blocked() {
    let app = app("20").await;
    sqlx::query(
        "INSERT INTO urls (id, random_key, default_fallback_url, hashed_value)
        VALUES
            (132, 'AZ', 'https://example.com/legacy-2char', 'fixture-legacy-2char-hash'),
            (12345, 'AbXy', 'https://example.com/existing-key', 'fixture-old-hash')
        ON CONFLICT (id) DO NOTHING",
    )
    .execute(&app.db)
    .await
    .unwrap();
    assert_eq!(app.get("/A28Z").await.status(), StatusCode::FOUND);
    assert_eq!(app.get("/A28X").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(app.get("/Ab3D7Xy").await.status(), StatusCode::FOUND);
    assert_eq!(app.get("/Ab3D7XY").await.status(), StatusCode::NOT_FOUND);

    let unsafe_key = app
        .insert_legacy("javascript:0", &destination(), true)
        .await;
    assert_eq!(
        app.get(&format!("/{unsafe_key}")).await.status(),
        StatusCode::NOT_FOUND
    );
    app.finish().await;
}

#[tokio::test]
async fn inactive_row_does_not_block_recreation() {
    let app = app("20").await;
    let url = destination();
    let hash = LinkFields {
        default_fallback_url: Some(url.clone()),
        ..LinkFields::default()
    }
    .dedupe_hash()
    .unwrap();
    let inactive = app.insert_legacy(&url, &hash, false).await;
    assert_eq!(
        app.get(&format!("/{inactive}")).await.status(),
        StatusCode::NOT_FOUND
    );
    let created = app.create(json!({"defaultFallbackUrl": url})).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_ne!(key(created).await, inactive);
    app.finish().await;
}

#[tokio::test]
async fn malformed_paths_return_json_not_found() {
    let app = app("20").await;
    for path in [
        "/favicon.ico",
        "/robots.txt",
        "/ab",
        "/A0Z",
        "/Ab00001Xy",
        "/Ab000000000001Xy",
        "/%FF",
    ] {
        let response = app.get(path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body(response).await, json!({"error": "URL not found"}));
    }
    app.finish().await;
}

#[tokio::test]
async fn health_and_ready_report_database_status() {
    let app = app("20").await;
    let health = app.get("/health").await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(body(health).await["status"], "ok");

    let ready = app.get("/ready").await;
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(
        body(ready).await,
        json!({"status": "ok", "database": "connected"})
    );
    app.finish().await;
}

#[tokio::test]
async fn l1_cache_serves_link_after_db_row_removed() {
    let app = app("20").await;
    let url = destination();
    let short_key = key(app.create(json!({"defaultFallbackUrl": url})).await).await;
    let (id, _) = url_shortener::short_key::parse_short_key(&short_key).unwrap();
    sqlx::query("DELETE FROM urls WHERE id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();

    // Creation warmed L1, so the click still succeeds from RAM without hitting PG.
    let response = app.get(&format!("/{short_key}")).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()[header::LOCATION], url);
    app.finish().await;
}

#[tokio::test]
async fn write_rate_limit_scopes_to_create_and_leaves_reads_unthrottled() {
    let app = app("2").await;
    let payload = json!({"defaultFallbackUrl": destination()}).to_string();
    assert_eq!(
        app.post(payload.clone(), &[]).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        app.post(payload.clone(), &[]).await.status(),
        StatusCode::OK
    );
    let limited = app.post(payload, &[]).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(body(limited).await, json!({"error": "Too many requests"}));

    // Index page, probes, and redirects remain unaffected.
    assert_eq!(app.get("/").await.status(), StatusCode::OK);
    assert_eq!(app.get("/health").await.status(), StatusCode::OK);
    assert_eq!(app.get("/ready").await.status(), StatusCode::OK);
    assert_eq!(app.get("/Ab00001Xy").await.status(), StatusCode::NOT_FOUND);
    app.finish().await;
}

#[tokio::test]
async fn forwarded_ip_is_used_only_from_a_trusted_peer() {
    let app = app("1").await;
    let payload = json!({"defaultFallbackUrl": destination()}).to_string();
    let send = |peer: [u8; 4], forwarded: &str| {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/urls")
            .header(header::CONTENT_TYPE, "application/json")
            .header("cf-connecting-ip", forwarded)
            .body(Body::from(payload.clone()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((peer, 12_345))));
        app.router.clone().oneshot(request)
    };

    // Trusted private peer (Cloudflare Tunnel on Docker network) honors CF-Connecting-IP.
    assert_eq!(
        send([172, 22, 0, 3], "203.0.113.10")
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        send([172, 22, 0, 3], "203.0.113.11")
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    // Direct public peer cannot spoof CF-Connecting-IP to evade rate limiting.
    assert_eq!(
        send([198, 51, 100, 1], "203.0.113.12")
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send([198, 51, 100, 1], "203.0.113.13")
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    app.finish().await;
}

#[tokio::test]
async fn normalizes_fields_before_hashing() {
    let app = app("20").await;
    let suffix = destination();
    let first = app
        .create(json!({"defaultFallbackUrl": format!("  {suffix}  "), "ogTitle": "  Title  "}))
        .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = app
        .create(json!({"defaultFallbackUrl": suffix, "ogTitle": "Title", "ogDescription": "   "}))
        .await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(key(first).await, key(second).await);
    app.finish().await;
}

#[tokio::test]
async fn concurrent_duplicate_creates_get_one_key() {
    let app = app("20").await;
    let payload = json!({"defaultFallbackUrl": destination()});
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let router = app.router.clone();
        let body_str = payload.to_string();
        tasks.push(tokio::spawn(async move {
            let req = Request::builder()
                .method("POST")
                .uri("/v1/urls")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body_str))
                .unwrap();
            let response = router.oneshot(req).await.unwrap();
            (response.status(), key(response).await)
        }));
    }
    let mut keys = std::collections::HashSet::new();
    let mut created = 0;
    for task in tasks {
        let (status, short_key) = task.await.unwrap();
        if status == StatusCode::CREATED {
            created += 1;
        } else {
            assert_eq!(status, StatusCode::OK);
        }
        keys.insert(short_key);
    }
    assert_eq!(created, 1);
    assert_eq!(keys.len(), 1);
    app.finish().await;
}
