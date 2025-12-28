# URL Shortener Service

[한국어](README.md) | [English](README.en.md)

🚀 **Demo Site:** [https://u.lou2.kr](https://u.lou2.kr)

![demo site](docs/screenshot.png)

## Introduction

A high-performance URL shortening service built with Rust. Features include deep link handling, platform-specific redirects, JWT authentication, and webhook notifications.

## Key Features

| Feature | Description |
|---------|-------------|
| **URL Shortening** | Collision-free unique short URLs using Base62 encoding |
| **Deep Links** | iOS/Android app deep links with platform-specific fallback URLs |
| **OG Tags** | Open Graph metadata for social media link previews |
| **Webhooks** | Real-time notifications when URLs are accessed (with concurrency control) |
| **Redis Caching** | High-speed responses with MessagePack serialization |
| **Rate Limiting** | SmartIP-based request throttling to prevent API abuse |
| **Multi-Compression** | Brotli, Gzip, and Zstd compression for optimized response sizes |

## Tech Stack

| Area | Technology |
|------|------------|
| Language | Rust 2021 Edition |
| Web Framework | Axum 0.8 |
| Async Runtime | Tokio |
| Database | PostgreSQL (SQLx) |
| Cache | Redis (deadpool-redis) |
| Cache Serialization | MessagePack (rmp-serde) |
| Template Engine | Askama |
| Authentication | JWT (jsonwebtoken) |
| Hashing | xxHash (xxh3_128) |
| Memory Allocator | mimalloc |
| Rate Limiting | tower_governor |
| Error Tracking | Sentry |

## Architecture

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│   Client    │────▶│  API Server │────▶│  PostgreSQL │
│  (Browser)  │     │   (Axum)    │     │  (SQLx)     │
└─────────────┘     └──────┬──────┘     └─────────────┘
                          │
                          ▼
                   ┌─────────────┐
                   │    Redis    │
                   │   (Cache)   │
                   └─────────────┘
```

## Short Key Generation

1. **Unique ID Generation**: Assigns a unique numeric ID when stored in database
2. **Base62 Encoding**: Converts numeric ID to a short string using Base62
3. **Random Prefix/Suffix**: Adds 4-character random key (2 chars prefix + 2 chars suffix) for unpredictability

```
Example: Random key "AbXy" → Prefix "Ab" + Base62 of ID 12345 "3D7" + Suffix "Xy" → "Ab3D7Xy"
```

**Benefits:**
- No collisions (based on database ID)
- Random prefix and suffix enhance protection against sequential key guessing
- Consistent performance regardless of database size

## Getting Started

### Prerequisites

- Rust 1.75+
- PostgreSQL
- Redis

### Installation

```bash
# Clone repository
git clone https://github.com/lee-lou2/url-shortener.git
cd url-shortener

# Configure environment
cp .env.example .env
# Edit .env file

# Run
cargo run

# Or release build
cargo run --release
```

### Environment Variables

```env
# Server
SERVER_PORT=3000
CORS_ORIGINS=*

# Database
DB_HOST=localhost
DB_PORT=5432
DB_USER=postgres
DB_PASSWORD=postgres
DB_NAME=postgres
DB_MAX_CONNECTIONS=20
DB_MIN_CONNECTIONS=2

# Redis
REDIS_HOST=localhost
REDIS_PORT=6379
REDIS_PASSWORD=
CACHE_TTL_SECS=3600

# JWT
JWT_SECRET=your-secret-key
JWT_EXPIRATION_HOURS=24

# Rate Limiting
RATE_LIMIT_PER_SECOND=10
RATE_LIMIT_BURST_SIZE=50

# Webhook
WEBHOOK_TIMEOUT_SECS=10
WEBHOOK_MAX_CONCURRENT=100

# Migrations
RUN_MIGRATIONS=true

# Sentry (optional)
SENTRY_DSN=
SENTRY_TRACES_SAMPLE_RATE=0.1
```

### Docker

```bash
docker build -t url-shortener .
docker run -p 3000:3000 --env-file .env url-shortener
```

## API

### `GET /`
Renders the main page and issues a guest JWT token.

### `POST /v1/urls`
Creates a new shortened URL.

**Authentication:** `Authorization: Bearer <token>` or cookie

**Request:**
```json
{
  "defaultFallbackUrl": "https://example.com",
  "iosDeepLink": "myapp://path",
  "iosFallbackUrl": "https://apps.apple.com/app/myapp",
  "androidDeepLink": "myapp://path",
  "androidFallbackUrl": "https://play.google.com/store/apps/details?id=com.myapp",
  "webhookUrl": "https://your-server.com/webhook",
  "ogTitle": "Title",
  "ogDescription": "Description",
  "ogImageUrl": "https://example.com/image.jpg"
}
```

**Response:**
```json
{
  "message": "URL created successfully",
  "short_key": "Ab3D7Xy"
}
```

### `GET /{short_key}`
Redirects the short URL to the original URL.
- Checks Redis cache → Falls back to DB on cache miss
- Detects platform and handles deep links/fallback URLs
- Calls webhook asynchronously (with Semaphore concurrency control)

## Development

### Build & Test

```bash
# Development mode
cargo run

# Release mode
cargo run --release

# Run tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Lint
cargo clippy

# Format code
cargo fmt
```

### Project Structure

```
src/
├── main.rs           # Entry point, server bootstrap
├── lib.rs            # Library crate
├── error.rs          # Centralized error handling
├── api/              # HTTP API layer
├── config/           # Environment configuration
├── models/           # Data models
└── utils/            # Utility functions

tests/
└── integration_test.rs  # Integration tests

views/                # HTML templates (Askama)
migrations/           # SQL migrations (SQLx)
```

## License

MIT License
