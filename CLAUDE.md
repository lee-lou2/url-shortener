# CLAUDE.md

> Comprehensive guide for AI coding assistants working on this codebase.

---

## Project Overview

**url-shortener** is a high-performance URL shortening service written in Rust. It provides collision-free short URL generation, deep linking for mobile apps (iOS/Android), platform-specific redirects, OG meta tag support, webhook notifications, and JWT-based authentication.

### Key Features

| Feature | Description |
|---------|-------------|
| **URL Shortening** | Collision-free short key generation using Base62 encoding + DB ID |
| **Deep Links** | iOS/Android deep link support with platform-specific fallback URLs |
| **OG Tags** | Social media preview support (title, description, image) |
| **Webhooks** | Async notifications on URL access with semaphore-based concurrency control |
| **Caching** | Redis caching with MessagePack serialization |
| **Authentication** | JWT-based API authentication (cookie + header support) |
| **Rate Limiting** | SmartIP-based request throttling |
| **Error Tracking** | Sentry integration for production error monitoring |

---

## Tech Stack

| Category | Technology | Version |
|----------|------------|---------|
| **Language** | Rust | 2021 Edition |
| **Web Framework** | Axum | 0.8 |
| **Async Runtime** | Tokio | 1.43 |
| **Database** | PostgreSQL + SQLx | 0.8 |
| **Cache** | Redis + deadpool-redis | 0.18 |
| **Serialization** | Serde, MessagePack (rmp-serde) | 1.0, 1.3 |
| **Templates** | Askama | 0.12 |
| **Auth** | jsonwebtoken | 9.3 |
| **Validation** | validator | 0.19 |
| **Logging** | tracing + tracing-subscriber | 0.1, 0.3 |
| **Hashing** | xxhash-rust (xxh3_128) | 0.8 |
| **Memory Allocator** | mimalloc | 0.1 |

---

## Architecture

```
                    ┌─────────────────┐
                    │     Client      │
                    │ (Browser/Mobile)│
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │     Router      │
                    │ (Axum Routes)   │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  Rate Limiter   │
                    │(tower_governor) │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │  JWT Middleware │
                    │  (Auth Check)   │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │    Handlers     │
                    │ (Business Logic)│
                    └───┬─────────┬───┘
                        │         │
            ┌───────────▼──┐  ┌───▼───────────┐
            │    Redis     │  │  PostgreSQL   │
            │   (Cache)    │  │  (Database)   │
            └──────────────┘  └───────────────┘
                        │
               ┌────────▼────────┐
               │    Webhooks     │
               │ (Async/Spawned) │
               └─────────────────┘
```

### Layered Architecture

- **API Layer** (`src/api/`): HTTP routing, handlers, middlewares, request/response schemas
- **Config Layer** (`src/config/`): Environment configuration, connection pool initialization
- **Model Layer** (`src/models/`): Domain entities, repository pattern for data access
- **Utility Layer** (`src/utils/`): Reusable utilities (JWT, Base62 encoding, random generation)
- **Error Layer** (`src/error.rs`): Centralized error handling with `AppError` enum

---

## Project Structure

```
src/
├── main.rs                 # Entry point, server bootstrap, graceful shutdown
├── lib.rs                  # Library crate exports
├── error.rs                # Centralized error types (AppError, AppResult)
├── api/
│   ├── mod.rs              # API module exports
│   ├── handlers.rs         # HTTP handlers (index, create_short_url, redirect)
│   ├── routes.rs           # Route definitions + middleware stack
│   ├── schemas.rs          # Request/Response DTOs with validation
│   ├── middlewares.rs      # JWT auth middleware (AuthUser extraction)
│   └── state.rs            # AppState (DB pool, Redis pool)
├── config/
│   ├── mod.rs              # Config module exports
│   ├── env.rs              # Environment variable loading (APP_CONFIG singleton)
│   ├── db.rs               # PostgreSQL connection pool (OnceCell)
│   └── cache.rs            # Redis connection pool (OnceCell)
├── models/
│   ├── mod.rs              # Models module exports
│   └── url.rs              # Url entity, UrlCacheData, UrlRepository
└── utils/
    ├── mod.rs              # Utils module exports
    ├── jwt.rs              # JWT generation/parsing
    ├── rand.rs             # Random alphanumeric string generation
    └── short_key.rs        # Base62 encoding/decoding for short keys

tests/
└── integration_test.rs     # Integration tests (runnable without DB)

migrations/                 # SQLx migration files
views/                      # Askama HTML templates
```

---

## Code Style Guide

### Rustfmt Configuration

The project uses `rustfmt.toml` with these settings:

| Setting | Value | Description |
|---------|-------|-------------|
| `max_width` | 100 | Maximum line width |
| `tab_spaces` | 4 | Indentation size |
| `newline_style` | Unix | Line ending style (LF) |
| `imports_granularity` | Module | Group imports by module |
| `group_imports` | StdExternalCrate | Order: std, external, crate |
| `fn_args_layout` | Tall | Function args on separate lines when long |
| `wrap_comments` | true | Wrap long comments |
| `format_code_in_doc_comments` | true | Format code blocks in docs |

### Clippy Configuration

Strict linting is enforced via `Cargo.toml`:

```toml
[lints.rust]
unsafe_code = "forbid"      # No unsafe code allowed

[lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
nursery = { level = "warn", priority = -1 }

# Allowed patterns
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
struct_field_names = "allow"
similar_names = "allow"
```

### Import Order

Always order imports in this sequence:

```rust
// 1. Standard library
use std::sync::Arc;

// 2. External crates
use axum::Router;
use tokio::sync::Semaphore;

// 3. Internal modules
use crate::api::AppState;
use crate::config::APP_CONFIG;
```

---

## Naming Conventions

### General Rules

| Item | Convention | Example |
|------|------------|---------|
| **Crates/Modules** | `snake_case` | `url_shortener`, `short_key` |
| **Types/Structs/Enums** | `PascalCase` | `AppError`, `UrlCacheData` |
| **Traits** | `PascalCase` | `ValidationErrorExt` |
| **Functions/Methods** | `snake_case` | `create_short_url`, `find_by_id` |
| **Constants** | `SCREAMING_SNAKE_CASE` | `APP_CONFIG`, `DB_POOL` |
| **Variables** | `snake_case` | `short_key`, `random_key` |

### Specific Naming Patterns

| Pattern | Usage | Example |
|---------|-------|---------|
| `*_handler` | HTTP handler functions | `index_handler`, `redirect_to_original_handler` |
| `*Request` | Request DTOs | `CreateShortUrlRequest` |
| `*Response` | Response DTOs | `CreateShortUrlResponse` |
| `*Template` | Askama templates | `IndexTemplate`, `RedirectTemplate` |
| `*Repository` | Data access objects | `UrlRepository` |
| `gen_*` | Generation functions | `gen_token`, `gen_rand_str` |
| `parse_*` | Parsing functions | `parse_token` |
| `init_*` | Initialization functions | `init_db`, `init_cache` |
| `close_*` | Cleanup functions | `close_cache` |
| `find_*` | Query functions | `find_by_id`, `find_by_hashed_value` |

### Environment Variables

| Prefix | Category | Examples |
|--------|----------|----------|
| `DB_*` | Database | `DB_HOST`, `DB_PORT`, `DB_USER`, `DB_PASSWORD`, `DB_NAME` |
| `REDIS_*` | Redis | `REDIS_HOST`, `REDIS_PORT`, `REDIS_PASSWORD` |
| `JWT_*` | Authentication | `JWT_SECRET`, `JWT_EXPIRATION_HOURS` |
| `SERVER_*` | Server | `SERVER_PORT` |
| `CACHE_*` | Caching | `CACHE_TTL_SECS` |
| `RATE_LIMIT_*` | Rate limiting | `RATE_LIMIT_PER_SECOND`, `RATE_LIMIT_BURST_SIZE` |
| `WEBHOOK_*` | Webhooks | `WEBHOOK_MAX_CONCURRENT` |

---

## Comment Style Guide

### Documentation Comments

Use `///` for public API documentation:

```rust
/// Generates a JWT token for the given subject.
///
/// # Arguments
/// * `subject` - User identifier (typically user ID or "guest")
///
/// # Returns
/// The JWT token string on success
///
/// # Errors
/// Returns `AppError::Jwt` if token generation fails
pub fn gen_token(subject: &str) -> AppResult<String> {
    // implementation
}
```

### Module-Level Documentation

Use `//!` at the top of modules:

```rust
//! Centralized error handling module.
//!
//! This module defines the `AppError` enum and `AppResult` type alias
//! used throughout the application for consistent error handling.
```

### Inline Comments

- Use sparingly, only for non-obvious logic
- Place on the line above the code, not at the end

```rust
// Disable connection validation for performance (pool handles reconnection)
.test_before_acquire(false)
```

### Section Separators (in tests)

```rust
// ============ Category Tests ============
```

### Warning Comments

Use emoji for security/important warnings:

```rust
tracing::warn!(
    "⚠️  JWT_SECRET not set - using insecure default. \
     Set RUST_ENV=production to enforce security requirements."
);
```

---

## Testing Guide

### Test Structure

Tests are organized in two locations:

1. **Unit Tests**: Inline `#[cfg(test)]` modules within each source file
2. **Integration Tests**: Separate `tests/integration_test.rs` file

### Test Naming Convention

```rust
#[test]
fn test_<function_name>_<scenario>() {
    // test implementation
}

// Examples:
fn test_gen_token_valid_subject()
fn test_merge_short_key_roundtrip()
fn test_app_error_bad_request_display()
```

### Async Tests

Use `#[tokio::test]` for async functions:

```rust
#[tokio::test]
async fn test_handler_returns_correct_status() {
    let response = some_async_function().await;
    assert_eq!(response.status(), StatusCode::OK);
}
```

### Test Categories

| Category | Coverage Focus |
|----------|----------------|
| **Error Tests** | Error display, HTTP status mapping, JSON serialization |
| **Schema Tests** | Validation, deserialization, serialization |
| **Utility Tests** | JWT, short key encoding, random generation |
| **Repository Tests** | Data conversion, caching, roundtrip |
| **Integration Tests** | End-to-end flows without database |

### Test Best Practices

1. **Test edge cases**: Empty strings, Unicode, special characters, boundary values
2. **Test roundtrips**: Encode → Decode → Encode should produce same result
3. **Use descriptive assertions**: Include context in failure messages
4. **Keep tests independent**: No shared mutable state between tests
5. **Test error conditions**: Verify error types and messages

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run tests in a specific module
cargo test module_name::
```

---

## Error Handling

### AppError Enum

All errors should use the centralized `AppError` type:

```rust
pub enum AppError {
    BadRequest(String),      // 400 - Client error
    Unauthorized(String),    // 401 - Auth failure
    NotFound(String),        // 404 - Resource not found
    Validation(String),      // 400 - Input validation failure
    Internal(String),        // 500 - Server error
    Database(sqlx::Error),   // 500 - DB error (auto-converted)
    Redis(...),              // 500 - Cache error (auto-converted)
    Jwt(...),                // 401 - JWT error (auto-converted)
    Template(...),           // 500 - Render error (auto-converted)
    Json(...),               // 400 - JSON parse error (auto-converted)
    HttpClient(...),         // 500 - HTTP client error (auto-converted)
}
```

### Error Handling Pattern

```rust
// Use ? operator with auto-conversion
let url = UrlRepository::find_by_id(&state.db, id).await?;

// Manual error creation
if url.is_none() {
    return Err(AppError::NotFound("URL not found".to_string()));
}

// Validation error conversion
body.validate().map_err(|e| e.to_validation_error())?;
```

### AppResult Type Alias

```rust
pub type AppResult<T> = Result<T, AppError>;
```

---

## Common Patterns

### Handler Pattern

```rust
pub async fn handler_name(
    State(state): State<AppState>,                    // Shared state
    Extension(auth_user): Extension<AuthUser>,        // Auth (if required)
    Json(body): Json<RequestType>,                    // Request body
) -> AppResult<impl IntoResponse> {
    // 1. Validate input
    body.validate().map_err(|e| e.to_validation_error())?;

    // 2. Business logic
    let result = Repository::do_something(&state.db, &body).await?;

    // 3. Return response
    Ok(Json(ResponseType { ... }))
}
```

### Repository Pattern

```rust
impl UrlRepository {
    pub async fn find_by_id(pool: &PgPool, id: i64) -> AppResult<Option<Url>> {
        sqlx::query_as!(
            Url,
            r#"
            SELECT id, random_key, default_fallback_url, ...
            FROM urls
            WHERE id = $1 AND deleted_at IS NULL
            "#,
            id
        )
        .fetch_optional(pool)
        .await
        .map_err(AppError::from)
    }
}
```

### Cache Pattern

```rust
// 1. Try cache first
let cache_key = format!("url:{id}");
if let Ok(cached) = redis_conn.get::<_, Vec<u8>>(&cache_key).await {
    if let Ok(data) = rmp_serde::from_slice::<UrlCacheData>(&cached) {
        return Ok(Some(data));
    }
}

// 2. Fallback to database
let url = UrlRepository::find_by_id(pool, id).await?;

// 3. Update cache
if let Some(ref url) = url {
    if let Ok(serialized) = rmp_serde::to_vec(url) {
        let _: Result<(), _> = redis_conn.setex(&cache_key, ttl, serialized).await;
    }
}
```

### Short Key Encoding

```rust
// Structure: [prefix(2)][base62_id][suffix(2)]
// Example: "Ab" + "3D7" + "Xy" = "Ab3D7Xy"

// Merge: Combine random_key with DB ID
let short_key = merge_short_key(id, &random_key)?;

// Split: Extract ID and random_key from short_key
let (extracted_id, extracted_random_key) = split_short_key(&short_key)?;
```

### Webhook Pattern (Fire-and-Forget)

```rust
// Non-blocking webhook with semaphore concurrency control
pub fn spawn_webhook_task(webhook_url: &str, short_key: &str, user_agent: &str) {
    let permit = match WEBHOOK_SEMAPHORE.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return,  // Skip if at capacity
    };

    tokio::spawn(async move {
        let _permit = permit;  // Hold until complete
        send_webhook_internal(&url, &key, &ua).await;
    });
}
```

---

## Key Constants

| Constant | Value | Location | Description |
|----------|-------|----------|-------------|
| `SHORT_KEY_MIN_LEN` | 5 | `short_key.rs` | Minimum short key length |
| `RANDOM_KEY_LEN` | 4 | `short_key.rs` | Random key total length |
| `RAND_PREFIX_LEN` | 2 | `short_key.rs` | Prefix portion of random key |
| `RAND_SUFFIX_LEN` | 2 | `short_key.rs` | Suffix portion of random key |
| `MIN_SECRET_LENGTH` | 32 | `jwt.rs` | Minimum JWT secret length |

---

## Database Schema

### `urls` Table

| Column | Type | Description |
|--------|------|-------------|
| `id` | BIGSERIAL PK | Auto-increment primary key |
| `random_key` | VARCHAR(4) | Random key (prefix 2 + suffix 2) |
| `default_fallback_url` | TEXT NOT NULL | Default redirect URL |
| `ios_deep_link` | TEXT | iOS deep link URI |
| `ios_fallback_url` | TEXT | iOS fallback URL |
| `android_deep_link` | TEXT | Android deep link URI |
| `android_fallback_url` | TEXT | Android fallback URL |
| `hashed_value` | TEXT NOT NULL | xxHash for duplicate detection |
| `webhook_url` | TEXT | Webhook notification URL |
| `og_title` | VARCHAR(255) | OG meta title |
| `og_description` | TEXT | OG meta description |
| `og_image_url` | TEXT | OG meta image URL |
| `is_active` | BOOLEAN | Active status flag |
| `created_at` | TIMESTAMPTZ | Creation timestamp |
| `deleted_at` | TIMESTAMPTZ | Soft delete timestamp |

### Key Indexes

| Index | Type | Purpose |
|-------|------|---------|
| `idx_urls_hashed_value_unique` | Partial unique | Duplicate prevention |
| `idx_urls_id_active` | Partial | ID lookup optimization |
| `idx_urls_is_active_partial` | Partial | Active URL filtering |

---

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|:----:|-------------|
| `GET` | `/` | No | Main page + guest JWT issuance |
| `POST` | `/v1/urls` | Yes | Create short URL |
| `GET` | `/{short_key}` | No | Redirect to original URL |
| `GET` | `/health` | No | Health check endpoint |
| `GET` | `/ready` | No | Readiness probe endpoint |

---

## Commands Reference

### Development

```bash
# Run development server
cargo run

# Run with hot reload (requires cargo-watch)
cargo watch -x run

# Check compilation without building
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Run all tests
cargo test

# Run tests with logging
RUST_LOG=debug cargo test -- --nocapture
```

### Production Build

```bash
# Build optimized release binary
cargo build --release

# Binary location
./target/release/url-shortener
```

### Docker

```bash
# Build image
docker build -t url-shortener .

# Run container
docker run -p 3000:3000 --env-file .env url-shortener
```

### Database Migrations

```bash
# Migrations run automatically on startup when RUN_MIGRATIONS=true
# Manual migration (requires sqlx-cli)
sqlx migrate run
```

---

## AI Coding Guidelines

### DO's

1. **Always run checks after changes**:
   ```bash
   cargo fmt && cargo clippy -- -D warnings && cargo test
   ```

2. **Use existing error types**: Leverage `AppError` variants, don't create new error enums

3. **Follow the repository pattern**: Add data access methods to existing repository structs

4. **Use `?` operator**: For error propagation with auto-conversion

5. **Use `tracing` macros**: For logging (`tracing::info!`, `tracing::error!`, etc.)

6. **Use `APP_CONFIG`**: For all configuration values

7. **Add validation**: Use `validator` derive macros on request DTOs

8. **Write tests**: Add unit tests for new functionality

9. **Use `Cow<'static, str>`**: To minimize string allocations where applicable

10. **Handle async properly**: Use `tokio::spawn` for fire-and-forget tasks

### DON'Ts

1. **No `.unwrap()` or `.expect()`**: Always use `?` or proper error handling

2. **No `unsafe` code**: Forbidden by lint configuration

3. **No ignoring Clippy warnings**: Fix all warnings before committing

4. **No blocking in async**: Don't use `std::thread::sleep` in async context

5. **No hardcoded config**: Use environment variables via `APP_CONFIG`

6. **No `println!`**: Use `tracing` macros instead

7. **No bypassing auth**: Protected routes must go through `auth_middleware`

8. **No SQL injection**: Always use parameterized queries via SQLx

9. **No committing secrets**: Keep credentials in environment variables

10. **No new dependencies without justification**: Evaluate necessity carefully

---

## Security Checklist

- [ ] Input validation with `validator` crate
- [ ] SQLx parameterized queries (SQL injection prevention)
- [ ] Protected routes through `auth_middleware`
- [ ] Secrets via environment variables (never hardcoded)
- [ ] Rate limiting with `tower_governor`
- [ ] Secure cookies in production (HTTPS-only, SameSite, HttpOnly)
- [ ] JWT secret minimum 32 characters in production
- [ ] Sentry error tracking for production monitoring

---

## Environment Variables Reference

| Variable | Default | Required | Description |
|----------|---------|:--------:|-------------|
| `SERVER_PORT` | 3000 | No | HTTP server port |
| `RUST_ENV` | development | No | Environment (development/production) |
| `RUST_LOG` | info | No | Log level filter |
| `DB_HOST` | localhost | No | PostgreSQL host |
| `DB_PORT` | 5432 | No | PostgreSQL port |
| `DB_USER` | postgres | No | Database username |
| `DB_PASSWORD` | - | Yes* | Database password |
| `DB_NAME` | url_shortener | No | Database name |
| `DB_MAX_CONNECTIONS` | 20 | No | Max DB pool connections |
| `REDIS_HOST` | localhost | No | Redis host |
| `REDIS_PORT` | 6379 | No | Redis port |
| `REDIS_PASSWORD` | - | No | Redis password |
| `REDIS_MAX_CONNECTIONS` | 50 | No | Max Redis pool connections |
| `CACHE_TTL_SECS` | 3600 | No | Cache TTL in seconds |
| `JWT_SECRET` | - | Yes** | JWT signing secret |
| `JWT_EXPIRATION_HOURS` | 24 | No | Token expiration hours |
| `RATE_LIMIT_PER_SECOND` | 10 | No | Requests per second limit |
| `RATE_LIMIT_BURST_SIZE` | 30 | No | Burst size for rate limit |
| `WEBHOOK_MAX_CONCURRENT` | 100 | No | Max concurrent webhooks |
| `RUN_MIGRATIONS` | true | No | Auto-run migrations on startup |
| `SENTRY_DSN` | - | No | Sentry error tracking DSN |
| `CORS_ORIGINS` | * | No | Allowed CORS origins |

\* Required for database connectivity
\** Required in production (`RUST_ENV=production`)

---

## Debugging Tips

| Setting | Purpose |
|---------|---------|
| `RUST_LOG=debug` | Enable debug-level logging |
| `RUST_LOG=sqlx=trace` | Log all SQL queries |
| `RUST_LOG=tower_http=trace` | Log HTTP request/response details |
| `redis-cli MONITOR` | Real-time Redis command monitoring |

---

## Performance Considerations

1. **Connection Pooling**: Both PostgreSQL and Redis use connection pools
2. **MessagePack**: Smaller serialization footprint than JSON for Redis cache
3. **xxHash**: Fast non-cryptographic hashing for deduplication
4. **mimalloc**: High-performance memory allocator (non-Windows)
5. **LTO**: Link-time optimization enabled for release builds
6. **Compression**: Gzip, Brotli, and Zstd support for responses
7. **Async Webhooks**: Non-blocking webhook calls with concurrency limits

---

*Last Updated: 2026-01-13*
