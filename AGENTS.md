# AGENTS.md

> Project Guide for AI Coding Agents

---

## 📋 Project Overview

**url-shortener** is a high-performance URL shortening service with deep link support, built in Rust.

### Core Features
- 🔗 **URL Shortening**: Collision-free unique short key generation using Base62 encoding
- 📱 **Deep Link Support**: iOS/Android app deep links with platform-specific fallback URLs
- 🖼️ **Open Graph Metadata**: OG tags for social media link previews
- 🔔 **Webhook Notifications**: Real-time notifications on URL access (Semaphore concurrency control)
- ⚡ **Redis Caching**: High-speed caching with MessagePack serialization
- 🔐 **JWT Authentication**: JSON Web Token-based API authentication
- 📊 **Sentry Integration**: Real-time error tracking and monitoring
- 🗜️ **Multi-Compression**: Brotli, GZIP, Zstd support
- 🚦 **Rate Limiting**: SmartIP-based API abuse prevention
- 🚀 **High-Performance Allocator**: Uses mimalloc

### Technology Stack
| Area | Technology |
|------|------------|
| Language | Rust 2021 Edition |
| Web Framework | Axum 0.8 |
| Async Runtime | Tokio |
| Database | PostgreSQL (SQLx) |
| Cache | Redis (deadpool-redis) |
| Cache Serialization | MessagePack (rmp-serde) |
| Template Engine | Askama |
| Serialization | Serde |
| Validation | Validator |
| Error Handling | thiserror, anyhow |
| Logging | tracing |
| Error Tracking | Sentry |
| Rate Limiting | tower_governor |
| Hashing | xxhash-rust (xxh3_128) |
| Memory Allocator | mimalloc |

---

## 🏗 Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Client    │────▶│  API Server │────▶│  PostgreSQL │
│  (Browser)  │     │   (Axum)    │     │  (SQLx)     │
└─────────────┘     └─────────────┘     └─────────────┘
       │                   │                   │
       │                   ▼                   │
       │            ┌─────────────┐            │
       │            │    Redis    │◀───────────┘
       │            │   (Cache)   │
       │            └─────────────┘
       │                   │
       ▼                   ▼
┌─────────────┐     ┌─────────────┐
│  Redirect   │     │   Webhook   │
│   Page      │     │  (Async)    │
└─────────────┘     └─────────────┘
```

### Data Flow

1. **URL Creation**: API request → Validation → xxHash duplicate check → DB save (ON CONFLICT) → Return short key
2. **URL Redirection**: Short key → Cache lookup (MessagePack) → DB fallback → Cache update → Webhook (async, Semaphore) → Render redirect page
3. **Authentication**: Request → JWT middleware → Token verification (header or cookie) → Claims extraction → Handler

---

## 📁 Project Structure

```
src/
├── main.rs                 # Entry point, initialization, server bootstrap
├── lib.rs                  # Library crate module exports
├── error.rs                # Centralized error types (AppError, AppResult, ValidationErrorExt)
├── api/
│   ├── mod.rs              # API module exports
│   ├── handlers.rs         # HTTP request handlers (IndexTemplate, RedirectTemplate)
│   ├── routes.rs           # Route definitions and middleware setup
│   ├── schemas.rs          # Request/Response DTOs and validation (validate_short_key)
│   ├── middlewares.rs      # JWT authentication middleware (AuthUser)
│   └── state.rs            # AppState definition (DB pool, Redis pool)
├── config/
│   ├── mod.rs              # Config module exports
│   ├── env.rs              # Environment variable loading (APP_CONFIG, get_env, get_env_parsed)
│   ├── db.rs               # PostgreSQL connection pool (OnceCell, init_db, close_db)
│   └── cache.rs            # Redis connection pool (OnceCell, init_cache, close_cache)
├── models/
│   ├── mod.rs              # Models module exports
│   └── url.rs              # Url, UrlCacheData, NewUrl, UrlRepository, CreateOrFindResult
└── utils/
    ├── mod.rs              # Utils module exports
    ├── jwt.rs              # JWT generation and parsing (gen_token, parse_token, Claims)
    ├── rand.rs             # Random string generation (gen_rand_str)
    └── short_key.rs        # Base62 encoding/decoding (merge_short_key, split_short_key)

tests/
└── integration_test.rs     # Integration tests (runnable without DB)

migrations/                 # SQL migration files (SQLx)
├── 20241228000000_create_urls_table.sql
├── 20241228000001_add_unique_constraint_and_partial_index.sql
└── 20241228000002_extend_random_key_length.sql

views/                      # HTML templates (Askama)
├── index.html              # Main page template
└── redirect.html           # Deep link handling redirect page
```

---

## 🔑 Core Modules

### `src/main.rs`
- Application entry point
- mimalloc global allocator setup (non-MSVC targets)
- tracing, Sentry, database, Redis initialization
- Database migration execution based on environment variables
- CORS, multi-compression (Brotli/GZIP/Zstd), rate limiting middleware setup
- Graceful shutdown handling (Ctrl+C, SIGTERM)

### `src/error.rs`
**Centralized Error Handling** - All errors are converted to `AppError`

```rust
#[derive(Error, Debug)]
pub enum AppError {
    BadRequest(String),      // 400
    Unauthorized(String),    // 401
    NotFound(String),        // 404
    Validation(String),      // 400
    Internal(String),        // 500
    Database(#[from] sqlx::Error),
    Redis(#[from] deadpool_redis::redis::RedisError),
    RedisPool(#[from] deadpool_redis::PoolError),
    Jwt(#[from] jsonwebtoken::errors::Error),
    Template(#[from] askama::Error),
    Json(#[from] serde_json::Error),
    HttpClient(#[from] reqwest::Error),
}

pub type AppResult<T> = Result<T, AppError>;

/// Helper trait to convert validation errors to AppError
pub trait ValidationErrorExt {
    fn to_validation_error(&self) -> AppError;
}
```

### `src/api/handlers.rs`
**HTTP Request Handlers** - All handlers follow this pattern:

```rust
pub async fn handler_name(
    State(state): State<AppState>,
    Json(body): Json<RequestType>,
) -> AppResult<impl IntoResponse> {
    // 1. Validation
    // 2. Business logic
    // 3. Return response
}
```

Key handlers:
- `index_handler`: Main page rendering + guest JWT issuance (INDEX_HTML is cached with Lazy)
- `create_short_url_handler`: URL creation (xxHash duplicate check, ON CONFLICT handling)
- `redirect_to_original_handler`: Redirection (cache lookup → DB fallback → async webhook call)

### `src/models/url.rs`
**URL Model and Repository** - Database operations using SQLx

```rust
/// URL creation or lookup result
pub enum CreateOrFindResult {
    Created(Url),    // Newly created
    Existing(Url),   // Existing URL returned
}

pub struct UrlRepository;

impl UrlRepository {
    pub async fn find_by_id(pool: &PgPool, id: i64) -> AppResult<Option<Url>>;
    pub async fn find_by_id_for_cache(pool: &PgPool, id: i64) -> AppResult<Option<UrlCacheData>>;
    pub async fn find_by_hashed_value(pool: &PgPool, hash: &str) -> AppResult<Option<Url>>;
    pub async fn create_or_find(pool: &PgPool, new_url: &NewUrl) -> AppResult<CreateOrFindResult>;
}
```

**Webhook Concurrency Control:**
```rust
/// Global Semaphore to limit concurrent webhook calls
static WEBHOOK_SEMAPHORE: Lazy<Arc<Semaphore>> =
    Lazy::new(|| Arc::new(Semaphore::new(APP_CONFIG.webhook_max_concurrent)));

impl UrlCacheData {
    /// Spawns webhook task asynchronously (concurrency controlled by Semaphore)
    pub fn spawn_webhook_task(self, short_key: Cow<'static, str>, user_agent: Cow<'static, str>);
}
```

---

## 🗄 Database Schema

### `urls` Table
| Column | Type | Description |
|--------|------|-------------|
| id | BIGSERIAL PK | Auto-increment ID |
| random_key | VARCHAR(4) | Random key for short URL security (2-char prefix + 2-char suffix) |
| ios_deep_link | TEXT | iOS app deep link URL |
| ios_fallback_url | TEXT | Fallback URL when iOS app not installed |
| android_deep_link | TEXT | Android app deep link URL |
| android_fallback_url | TEXT | Fallback URL when Android app not installed |
| default_fallback_url | TEXT NOT NULL | Default redirect URL (required) |
| hashed_value | TEXT NOT NULL | xxHash for duplicate prevention (128-bit) |
| webhook_url | TEXT | Webhook URL to call on access |
| og_title | VARCHAR(255) | Open Graph title |
| og_description | TEXT | Open Graph description |
| og_image_url | TEXT | Open Graph image URL |
| is_active | BOOLEAN | URL activation status |
| created_at | TIMESTAMPTZ | Creation timestamp |
| updated_at | TIMESTAMPTZ | Update timestamp |
| deleted_at | TIMESTAMPTZ | Soft delete timestamp |

### Indexes
| Index | Column | Purpose |
|-------|--------|---------|
| idx_urls_hashed_value_unique | hashed_value (UNIQUE, WHERE deleted_at IS NULL) | Duplicate prevention (partial index) |
| idx_urls_id_active | id (WHERE deleted_at IS NULL) | ID lookup optimization (partial index) |
| idx_urls_is_active_partial | is_active (WHERE deleted_at IS NULL AND is_active = true) | Active URL filtering (partial index) |
| idx_urls_deleted_at | deleted_at | Soft delete queries |

---

## 🌐 API Endpoints

| Method | Path | Auth | Handler | Description |
|--------|------|:----:|---------|-------------|
| GET | `/` | ❌ | `index_handler` | Main page and guest JWT issuance |
| POST | `/v1/urls` | ✅ | `create_short_url_handler` | Create short URL |
| GET | `/{short_key}` | ❌ | `redirect_to_original_handler` | Redirect to original URL |

---

## ⚙️ Environment Variables

### Server Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `SERVER_PORT` | ❌ | 3000 | Server port |
| `CORS_ORIGINS` | ❌ | * | CORS allowed origins (comma-separated) |

### Database Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `DB_HOST` | ❌ | localhost | PostgreSQL host |
| `DB_PORT` | ❌ | 5432 | PostgreSQL port |
| `DB_USER` | ❌ | postgres | PostgreSQL user |
| `DB_PASSWORD` | ❌ | postgres | PostgreSQL password |
| `DB_NAME` | ❌ | postgres | PostgreSQL database name |
| `DB_MAX_CONNECTIONS` | ❌ | 20 | Maximum connections |
| `DB_MIN_CONNECTIONS` | ❌ | 2 | Minimum connections |
| `DB_ACQUIRE_TIMEOUT_SECS` | ❌ | 5 | Connection acquire timeout (seconds) |
| `DB_IDLE_TIMEOUT_SECS` | ❌ | 600 | Idle connection timeout (seconds) |
| `DB_MAX_LIFETIME_SECS` | ❌ | 1800 | Connection maximum lifetime (seconds) |

### Redis Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `REDIS_HOST` | ❌ | localhost | Redis host |
| `REDIS_PORT` | ❌ | 6379 | Redis port |
| `REDIS_PASSWORD` | ❌ | - | Redis password |
| `CACHE_TTL_SECS` | ❌ | 3600 | Cache TTL (seconds) |

### Authentication Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `JWT_SECRET` | ✅ (production) | dev default | JWT signing secret (32+ chars recommended) |
| `JWT_EXPIRATION_HOURS` | ❌ | 24 | JWT expiration time (hours) |
| `RUST_ENV` | ❌ | development | Environment (JWT_SECRET required in production) |

### Rate Limiting
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `RATE_LIMIT_PER_SECOND` | ❌ | 10 | Requests per second limit |
| `RATE_LIMIT_BURST_SIZE` | ❌ | 50 | Burst request allowance |

### Webhook Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `WEBHOOK_TIMEOUT_SECS` | ❌ | 10 | Webhook request timeout (seconds) |
| `WEBHOOK_MAX_CONCURRENT` | ❌ | 100 | Maximum concurrent webhook requests (Semaphore) |

### Other Configuration
| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `SENTRY_DSN` | ❌ | - | Sentry DSN (error tracking) |
| `SENTRY_TRACES_SAMPLE_RATE` | ❌ | 0.1 | Sentry trace sampling rate |
| `RUN_MIGRATIONS` | ❌ | true | Run migrations on startup |

---

## 🔧 Development Environment

### Build and Run

```bash
# Development mode
cargo run

# Release mode
cargo run --release

# Watch mode (requires cargo-watch)
cargo watch -x run

# Run tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Lint
cargo clippy

# Code formatting
cargo fmt
```

### Key Constants

| Constant | Value | Location |
|----------|-------|----------|
| `SHORT_KEY_MIN_LEN` | 5 | short_key.rs (minimum length: prefix 2 + ID 1 + suffix 2) |
| `RANDOM_KEY_LEN` | 4 | short_key.rs (prefix 2 + suffix 2) |
| `RAND_PREFIX_LEN` | 2 | short_key.rs (prefix length) |
| `RAND_SUFFIX_LEN` | 2 | short_key.rs (suffix length) |
| `MIN_SECRET_LENGTH` | 32 | jwt.rs |

---

## 📝 Rust Code Style Guide

> This project follows the [Rust Official Style Guide](https://doc.rust-lang.org/stable/style-guide/) and [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/).

### rustfmt.toml Configuration

```toml
edition = "2021"
max_width = 100
tab_spaces = 4
newline_style = "Unix"
use_small_heuristics = "Default"

# Import configuration
imports_granularity = "Module"
group_imports = "StdExternalCrate"
reorder_imports = true
reorder_modules = true

# Function and struct formatting
fn_args_layout = "Tall"
struct_lit_single_line = true

# Comment formatting
wrap_comments = true
format_code_in_doc_comments = true
doc_comment_code_block_width = 80
```

### Lint Configuration

```toml
[lints.rust]
unsafe_code = "forbid"

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

### Naming Conventions

| Item | Style | Example |
|------|-------|---------|
| Crates/Modules | `snake_case` | `url_shortener`, `short_key` |
| Types/Traits | `PascalCase` | `UrlCacheData`, `AppError`, `ValidationErrorExt` |
| Functions/Methods | `snake_case` | `create_short_url_handler`, `find_by_id` |
| Constants | `SCREAMING_SNAKE_CASE` | `APP_CONFIG`, `DB_POOL`, `WEBHOOK_SEMAPHORE` |
| Variables/Parameters | `snake_case` | `db_pool`, `short_key` |
| Lifetimes | short lowercase | `'a`, `'de` |
| Type Parameters | single uppercase or `PascalCase` | `T`, `E`, `Item` |

### Module Documentation Comments

```rust
// ✅ Good: Concise one-liner
//! HTTP request handlers module.

// ❌ Bad: Unnecessarily verbose
//! This module contains all HTTP request handlers for the URL shortening service.
//! 
//! ## Features
//! - URL creation handling
//! - Redirect handling
//! ...
```

### Function Documentation Comments

```rust
// ✅ Good: Document when it adds value
/// Creates a new short URL with the provided request.
///
/// # Route
///
/// `POST /v1/urls`
pub async fn create_short_url_handler(...) -> AppResult<...>

// ✅ Good: Minimal docs for simple functions
/// Finds a URL by ID.
pub async fn find_by_id(pool: &PgPool, id: i64) -> AppResult<Option<Url>>
```

### Import Organization

```rust
// ✅ Good: std → external → internal order
use std::net::SocketAddr;
use std::time::Duration;

use axum::http::Method;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::api::{create_routes, AppState};
use crate::config::{init_cache, init_db, APP_CONFIG};
```

### Error Handling

```rust
// ✅ Good: Use thiserror for error types
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ✅ Good: Propagate with ? operator
let url = UrlRepository::find_by_id(&state.db, id).await?;

// ✅ Good: Use ok_or_else for Option to Result conversion
let url = url.ok_or_else(|| AppError::NotFound("URL not found".to_string()))?;

// ❌ Bad: Using unwrap in production code
let url = UrlRepository::find_by_id(&state.db, id).await.unwrap();
```

### Handler Function Naming

```rust
// ✅ Good: HTTP handlers have descriptive names with _handler suffix
pub async fn index_handler(...) -> impl IntoResponse
pub async fn create_short_url_handler(...) -> impl IntoResponse
pub async fn redirect_to_original_handler(...) -> impl IntoResponse

// ✅ Good: Internal functions without suffix
fn render_redirect_page(url_data: &UrlCacheData) -> AppResult<Response>
fn extract_token(request: &Request<Body>, jar: &CookieJar) -> Option<String>
```

---

## 🧪 Test Code Style Guide

### Test File Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_name_scenario() {
        // Arrange
        let input = "test_input";
        
        // Act
        let result = function_under_test(input);
        
        // Assert
        assert!(result.is_ok());
    }
}
```

### Test Function Naming

```rust
// ✅ Good: test_ prefix + function_name + scenario
#[test]
fn test_validate_short_key_valid() { }

#[test]
fn test_validate_short_key_too_short() { }

#[test]
fn test_validate_short_key_invalid_chars() { }

// ❌ Bad: Unclear or overly long names
#[test]
fn test1() { }

#[test]
fn test_that_when_validating_a_short_key_it_should_fail_if_too_short() { }
```

### Async Test Pattern

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_async_function() {
        // Arrange
        let db = setup_test_db().await;
        
        // Act
        let result = async_function(&db).await;
        
        // Assert
        assert!(result.is_ok());
    }
}
```

### Integration Tests

The `tests/integration_test.rs` file contains integration tests runnable without a DB:
- Full URL shortening flow tests
- JWT authentication flow tests
- Short key generation and validation tests
- Error handling tests
- MessagePack serialization tests

---

## 🤖 AI Agent Guidelines

### DO's (Recommended Practices)

1. **Always run `cargo check` or `cargo build`** after making changes to verify compilation
2. **Run `cargo clippy`** to catch common mistakes and follow Rust idioms
3. **Run `cargo fmt`** to ensure consistent code formatting
4. **Run `cargo test`** after changes to ensure no regressions
5. **Use existing error types** (`AppError`) instead of creating new ones
6. **Follow the existing patterns** in the codebase for consistency
7. **Use the repository pattern** (`UrlRepository`) for database operations
8. **Leverage the `?` operator** for error propagation
9. **Add new routes** in `src/api/routes.rs` following the existing structure
10. **Use `Cow<'static, str>`** for strings that might be static or owned
11. **Use `#[must_use]`** for functions returning important values that shouldn't be ignored
12. **Use `const fn`** for simple constructors (e.g., `AppState::new`)

### DON'Ts (Avoid These)

1. **DON'T use `.unwrap()` or `.expect()`** in production code - use `?` or proper error handling
2. **DON'T add `unsafe` code** - it's forbidden via lint configuration
3. **DON'T ignore Clippy warnings** - fix them or document why they're acceptable
4. **DON'T create new error types** without extending `AppError`
5. **DON'T bypass the authentication middleware** for protected routes
6. **DON'T use blocking operations** in async contexts - use tokio equivalents
7. **DON'T hardcode configuration values** - use environment variables via `APP_CONFIG`
8. **DON'T forget to add tests** for new functionality
9. **DON'T use `println!` for logging** - use the `tracing` crate macros (`tracing::info!`, etc.)
10. **DON'T create circular dependencies** between modules

### Common Patterns Reference

#### Adding a New Handler

```rust
// 1. Define request/response schemas in src/api/schemas.rs
#[derive(Debug, Deserialize, Validate)]
pub struct NewFeatureRequest {
    #[validate(length(min = 1, max = 100))]
    pub field: String,
}

#[derive(Debug, Serialize)]
pub struct NewFeatureResponse {
    pub result: String,
}

// 2. Implement handler in src/api/handlers.rs
pub async fn new_feature_handler(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(body): Json<NewFeatureRequest>,
) -> AppResult<impl IntoResponse> {
    body.validate().map_err(|e| e.to_validation_error())?;
    
    // Business logic here
    
    Ok(Json(NewFeatureResponse { result: "success".to_string() }))
}

// 3. Add route in src/api/routes.rs
.route("/v1/new-feature", post(new_feature_handler))
```

#### Adding a New Database Query

```rust
// In src/models/url.rs or a new model file
impl UrlRepository {
    pub async fn new_query(pool: &PgPool, param: &str) -> AppResult<Option<Url>> {
        sqlx::query_as!(
            Url,
            r#"
            SELECT id, random_key, default_fallback_url, ...
            FROM urls
            WHERE some_column = $1 AND deleted_at IS NULL
            "#,
            param
        )
        .fetch_optional(pool)
        .await
        .map_err(AppError::from)
    }
}
```

#### Caching Pattern

```rust
// Check cache first, then DB fallback
async fn get_with_cache(state: &AppState, key: &str) -> AppResult<Option<Data>> {
    // 1. Try cache
    if let Some(ref cache) = state.cache {
        if let Ok(mut conn) = cache.get().await {
            if let Ok(cached) = redis::cmd("GET").arg(key).query_async::<Vec<u8>>(&mut *conn).await {
                if let Ok(data) = rmp_serde::from_slice(&cached) {
                    return Ok(Some(data));
                }
            }
        }
    }
    
    // 2. Fallback to DB
    let data = Repository::find_by_key(&state.db, key).await?;
    
    // 3. Update cache if found
    if let (Some(ref data), Some(ref cache)) = (&data, &state.cache) {
        if let Ok(mut conn) = cache.get().await {
            if let Ok(serialized) = rmp_serde::to_vec(data) {
                let _: Result<(), _> = redis::cmd("SETEX")
                    .arg(key)
                    .arg(APP_CONFIG.cache_ttl_secs)
                    .arg(serialized)
                    .query_async(&mut *conn)
                    .await;
            }
        }
    }
    
    Ok(data)
}
```

### Debugging Tips

1. **Enable debug logging**: Set `RUST_LOG=debug` or `RUST_LOG=url_shortener=debug`
2. **Check SQL queries**: SQLx logs queries at trace level with `RUST_LOG=sqlx=trace`
3. **Inspect Redis operations**: Use `redis-cli MONITOR` for real-time command monitoring
4. **Test endpoints**: Use `curl` or tools like HTTPie for quick API testing
5. **Database inspection**: Connect with `psql` to verify data state

### Performance Considerations

1. **Use connection pooling**: Already configured via `sqlx::PgPool` and `deadpool_redis`
2. **Leverage caching**: Redis caching is available - use it for frequently accessed data
3. **Async all the way**: Never block the async runtime with sync operations
4. **Use `Cow<'_, str>`**: Avoid unnecessary string allocations
5. **Batch operations**: Use `sqlx::query_as!` with `fetch_all` for bulk reads

### Security Checklist

1. **Input validation**: Always validate user input using the `validator` crate
2. **SQL injection**: Use parameterized queries (SQLx enforces this)
3. **JWT verification**: All protected routes must go through `auth_middleware`
4. **Rate limiting**: Configured via `tower_governor` - adjust limits as needed
5. **Secrets management**: Never hardcode secrets - use environment variables

---

## 🚨 Known Limitations

1. **Single Instance**: Requires Redis cache for session sharing during horizontal scaling
2. **Cache Dependency**: Requires Redis for optimal performance (falls back to direct DB queries)
3. **Webhook Reliability**: Fire-and-forget approach (no retry mechanism, Semaphore limits concurrency)
4. **JWT Secret**: Must set `JWT_SECRET` environment variable in production

---

## 🤝 Contribution Guidelines

1. **Branch Naming**: `feature/feature-name`, `fix/bug-name`
2. **Commit Messages**: `[module] Change summary`
3. **Test Passing**: All `cargo test` must pass
4. **Clippy Passing**: No `cargo clippy` warnings (including nursery)
5. **Code Formatting**: Apply `cargo fmt`

### Pre-commit Checklist

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

---

## 📚 References

- [The Rust Programming Language Book](https://doc.rust-lang.org/book/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)
- [Clippy Lints](https://rust-lang.github.io/rust-clippy/master/)
- [Axum Documentation](https://docs.rs/axum/latest/axum/)
- [SQLx Documentation](https://docs.rs/sqlx/latest/sqlx/)
- [deadpool-redis Documentation](https://docs.rs/deadpool-redis/latest/deadpool_redis/)

---

*Last Updated: 2025-12-28*
