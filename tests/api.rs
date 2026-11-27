use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    body::{to_bytes, Body},
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
    config: Config,
}

fn destination() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let nonce: u64 = rand::random();
    format!("https://example.com/test/{time}-{nonce}")
}

async fn app(burst: &str, redis_port: Option<u16>) -> TestApp {
    let mut config = Config::from_lookup(|key| match key {
        "RUST_ENV" => Some("production".into()),
        "JWT_SECRET" => Some("local-test-signing-key-with-32-bytes".into()),
        "REDIS_HOST" => {
            Some(std::env::var("TEST_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into()))
        }
        "REDIS_PORT" => redis_port
            .map(|port| port.to_string())
            .or_else(|| std::env::var("TEST_REDIS_PORT").ok()),
        "REDIS_DB" => Some("14".into()),
        "RATE_LIMIT_BURST_SIZE" => Some(burst.into()),
        "RATE_LIMIT_PER_SECOND" => Some("1".into()),
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
        // Prefer the compose.yaml default (postgres/postgres@127.0.0.1/url_shortener),
        // falling back to local OS-user url_shortener_test when Postgres runs natively.
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
    let state = AppState::new(db.clone(), &config).await.unwrap();
    let router = create_routes(state.clone(), &config).unwrap();
    TestApp {
        router,
        state,
        db,
        config,
    }
}

impl TestApp {
    async fn get(&self, path: &str) -> Response {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        self.router.clone().oneshot(request).await.unwrap()
    }

    async fn cookie(&self) -> String {
        self.get("/").await.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }

    async fn post(&self, body: String, credentials: Option<(&str, &str)>) -> Response {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/urls")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some((name, value)) = credentials {
            request = request.header(name, value);
        }
        self.router
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }

    async fn create(&self, value: Value) -> Response {
        self.post(value.to_string(), Some(("cookie", &self.cookie().await)))
            .await
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
    assert!(headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains(&format!("'sha256-{digest}'")));
}

#[tokio::test]
async fn index_sets_guest_cookie_and_csp() {
    let app = app("20", None).await;
    let response = app.get("/").await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let cookie = headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with("token="));
    assert!(cookie.ends_with("; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400; Secure"));
    check_csp(&headers, &text(response).await);
    app.finish().await;
}

#[tokio::test]
async fn creation_requires_cookie_or_bearer() {
    let app = app("20", None).await;
    let payload = json!({"defaultFallbackUrl":destination()}).to_string();
    for creds in [None, Some(("authorization", "Bearer invalid"))] {
        let response = app.post(payload.clone(), creds).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body(response).await,
            json!({"error":"Invalid or missing token"})
        );
    }
    let cookie = app.cookie().await;
    let bearer = format!("bEaReR {}", cookie.strip_prefix("token=").unwrap());
    let response = app
        .post(payload.clone(), Some(("authorization", &bearer)))
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = app.post(payload, Some(("cookie", &cookie))).await;
    assert_eq!(response.status(), StatusCode::OK);
    app.finish().await;
}

#[tokio::test]
async fn invalid_urls_and_json_are_rejected() {
    let app = app("20", None).await;
    let cookie = app.cookie().await;
    for (name, value) in [
        ("defaultFallbackUrl", "javascript:0"),
        ("iosDeepLink", "data:text/plain,blocked"),
        ("webhookUrl", "http://127.0.0.1/"),
        ("webhookUrl", "http://169.254.169.254/"),
        ("androidFallbackUrl", "https://example.com/\r\n"),
    ] {
        let mut payload = json!({"defaultFallbackUrl":"https://example.com/"});
        payload[name] = json!(value);
        let response = app
            .post(payload.to_string(), Some(("cookie", &cookie)))
            .await;
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
        let response = app.post(payload, Some(("cookie", &cookie))).await;
        assert_eq!(response.status(), status);
        assert!(body(response).await["error"].is_string());
    }
    app.finish().await;
}

#[tokio::test]
async fn exact_dedupe_and_distinct_metadata() {
    let app = app("20", None).await;
    let url = destination();
    let created = app.create(json!({"defaultFallbackUrl":url})).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body(created).await;
    assert_eq!(
        created,
        json!({"created":true,"message":"URL shortened successfully","short_key":created["short_key"]})
    );
    let duplicate = app
        .create(json!({"defaultFallbackUrl":url,"iosDeepLink":"  "}))
        .await;
    assert_eq!(duplicate.status(), StatusCode::OK);
    assert_eq!(
        body(duplicate).await,
        json!({"created":false,"message":"URL already exists","short_key":created["short_key"]})
    );
    for payload in [
        json!({"defaultFallbackUrl":url,"ogTitle":"New title"}),
        json!({"defaultFallbackUrl":url,"webhookUrl":"https://example.net/hook"}),
    ] {
        let response = app.create(payload).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_ne!(key(response).await, created["short_key"].as_str().unwrap());
    }
    app.finish().await;
}

#[tokio::test]
async fn plain_redirect_uses_location_header() {
    let app = app("20", None).await;
    let url = destination();
    let key = key(app.create(json!({"defaultFallbackUrl":url})).await).await;
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
    let app = app("20", None).await;
    let key = key(app.create(json!({
        "defaultFallbackUrl":destination(), "iosDeepLink":"myapp://open", "ogTitle":r#""><em>unsafe</em>"#
    })).await).await;
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
    let app = app("20", None).await;
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
    let app = app("20", None).await;
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
    let created = app.create(json!({"defaultFallbackUrl":url})).await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_ne!(key(created).await, inactive);
    app.finish().await;
}

#[tokio::test]
async fn malformed_paths_return_json_not_found() {
    let app = app("20", None).await;
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
        assert_eq!(body(response).await, json!({"error":"URL not found"}));
    }
    app.finish().await;
}

#[tokio::test]
async fn redis_is_optional_when_unreachable_at_startup() {
    let unavailable = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let app = app("20", Some(unavailable.local_addr().unwrap().port())).await;
    let ready = app.get("/ready").await;
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(
        body(ready).await,
        json!({"status":"degraded","database":"connected","cache":"disconnected"})
    );
    let url = destination();
    let known_key = app.insert_legacy(&url, &url, true).await;
    let response = app.get(&format!("/{known_key}")).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()[header::LOCATION], url);
    app.finish().await;
}

#[tokio::test]
async fn cold_redirect_populates_redis_and_next_instance_uses_it() {
    let app = app("20", None).await;
    assert_eq!(
        body(app.get("/ready").await).await,
        json!({"status":"ok","database":"connected","cache":"connected"})
    );
    let url = destination();
    let known_key = app.insert_legacy(&url, &url, true).await;
    let path = format!("/{known_key}");
    assert_eq!(app.get(&path).await.status(), StatusCode::FOUND);

    let id = url_shortener::short_key::parse_short_key(&known_key)
        .unwrap()
        .0;
    let client = redis::Client::open(app.config.redis.clone().unwrap()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    let redis_key = format!("u:{id}");
    let stored: String = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let value: Option<String> = redis::cmd("GET")
                .arg(&redis_key)
                .query_async(&mut connection)
                .await
                .unwrap();
            if let Some(value) = value {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap()["defaultFallbackUrl"],
        url
    );
    let ttl: i64 = redis::cmd("TTL")
        .arg(&redis_key)
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!((1..=60).contains(&ttl));

    sqlx::query("UPDATE urls SET is_active = false WHERE id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();
    let other_db = app.config.connect_db().await.unwrap();
    let state = AppState::new(other_db, &app.config).await.unwrap();
    let router = create_routes(state.clone(), &app.config).unwrap();
    let request = Request::builder().uri(&path).body(Body::empty()).unwrap();
    assert_eq!(
        router.oneshot(request).await.unwrap().status(),
        StatusCode::FOUND
    );
    state.shutdown().await;
    app.finish().await;
}

#[tokio::test]
async fn write_rate_limit_does_not_include_redirects_or_probes() {
    let app = app("2", None).await;
    let created = app
        .create(json!({"defaultFallbackUrl":destination()}))
        .await;
    let path = format!("/{}", key(created).await);
    let limited = app.get("/").await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers()[header::RETRY_AFTER], "1");
    assert_eq!(body(limited).await, json!({"error":"Too many requests"}));
    for (path, status) in [
        ("/health", StatusCode::OK),
        ("/ready", StatusCode::OK),
        (&path, StatusCode::FOUND),
    ] {
        assert_eq!(app.get(path).await.status(), status);
    }
    app.finish().await;
}

#[test]
fn normalizes_fields_before_hashing() {
    let fields = LinkFields {
        default_fallback_url: Some(" https://BÜCHER.de/새 ".into()),
        ios_deep_link: Some(" myapp://home ".into()),
        og_description: Some("   ".into()),
        ..LinkFields::default()
    }
    .normalize()
    .unwrap();
    assert_eq!(
        fields.default_fallback_url.as_deref(),
        Some("https://xn--bcher-kva.de/%EC%83%88")
    );
    assert_eq!(fields.ios_deep_link.as_deref(), Some("myapp://home"));
    assert!(fields.og_description.is_none());
}

#[tokio::test]
async fn concurrent_duplicate_creates_get_one_key() {
    let app = app("30", None).await;
    let cookie = app.cookie().await;
    let payload = json!({"defaultFallbackUrl":destination()}).to_string();
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let router = app.router.clone();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/urls")
            .header(header::COOKIE, &cookie)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload.clone()))
            .unwrap();
        requests.spawn(async move { router.oneshot(request).await.unwrap() });
    }

    let mut created = 0;
    let mut keys = std::collections::HashSet::new();
    while let Some(response) = requests.join_next().await {
        let response = response.unwrap();
        match response.status() {
            StatusCode::CREATED => created += 1,
            StatusCode::OK => {}
            status => panic!("Unexpected creation status {status}"),
        }
        keys.insert(key(response).await);
    }
    assert_eq!(created, 1);
    assert_eq!(keys.len(), 1);
    app.finish().await;
}

#[tokio::test]
async fn forwarded_ip_is_used_only_from_a_trusted_peer() {
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;

    let app = app("1", None).await;
    for (peer, advertised, status) in [
        ("172.16.0.2:12345", "1.1.1.1", StatusCode::OK),
        ("172.16.0.2:12345", "8.8.8.8", StatusCode::OK),
        ("172.16.0.2:12345", "1.1.1.1", StatusCode::TOO_MANY_REQUESTS),
        ("203.0.113.2:12345", "1.1.1.1", StatusCode::OK),
        (
            "203.0.113.2:12345",
            "8.8.8.8",
            StatusCode::TOO_MANY_REQUESTS,
        ),
    ] {
        let mut request = Request::builder()
            .uri("/")
            .header("cf-connecting-ip", advertised)
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let response = app.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), status);
    }
    app.finish().await;
}
