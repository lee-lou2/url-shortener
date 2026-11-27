# URL Shortener

[한국어](README.md) | [English](README.en.md)

🚀 **Demo:** [https://u.lou2.kr](https://u.lou2.kr)

![URL Shortener demo UI](docs/screenshot.png)

A fast, **stateless** public URL shortener built with Rust (Axum 0.8 + PostgreSQL + `moka`).
It requires only PostgreSQL (no Redis, no session tokens), supports app deep links, platform fallbacks, Open Graph previews, and click webhooks, and redirects plain links in a single `302`.

## Features

- **Collision-free short keys** from the DB `id` (`prefix(2) + base62(id) + suffix(2)`, plus legacy 2-char key compatibility) — zero-allocation Base62 parsing before I/O, identical payloads reuse the existing link
- **Deep links** for iOS / iPadOS / Android with store fallbacks; automatically cancels the fallback timer on `visibilitychange`/`pagehide`/`blur`
- **Social previews**: Open Graph / Twitter Card tags with live form preview
- **Click webhooks**: async `POST` per visit, DNS-pinning SSRF defense, bounded concurrency (64), 5 s graceful drain on shutdown
- **In-process TinyLFU cache**: single `moka` L1 keyed by `i64 id` (60 s found / 15 s missing, singleflight, 32 MiB cap) — scales horizontally with zero Redis SPOF
- **Modern web security by default**: stateless W3C Fetch Metadata (`Sec-Fetch-Site`/`Origin`) CSRF guard, `http(s)` allow-list, hash-pinned SHA-256 CSP + `frame-ancestors 'none'`, peer-trusted `CF-Connecting-IP` GCRA rate limits

## Architecture

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/architecture-dark.png">
  <img alt="System architecture" src="docs/diagrams/architecture.png" width="960">
</picture>

| Area | Technology |
|------|------------|
| Web | Axum 0.8, Tower, Tokio |
| Data | PostgreSQL 15 + SQLx (embedded migrations run on boot) |
| Cache | In-process `moka` TinyLFU L1 (32 MiB byte cap, singleflight) |
| Abuse guard | W3C Fetch Metadata (`Sec-Fetch-Site`/`Origin`) + `tower_governor` GCRA |
| Pages | Plain HTML/CSS/JS in `views/` (zero build step, zero CDN); Askama renders redirects |
| Ops | Sentry, `tracing`, mimalloc (Linux/macOS), graceful shutdown |

### Redirect: `GET /{short_key}`

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/url-redirect-flow-dark.png">
  <img alt="Redirect flow" src="docs/diagrams/url-redirect-flow.png" width="960">
</picture>

| The link has | Response |
|--------------|----------|
| only a destination URL (webhook optional) | `302 Found` + `Location`, empty body |
| deep links, platform fallbacks, or Open Graph fields | `200 OK` HTML: OG tags + platform-aware redirect script |
| invalid, unknown, or deleted key | `404` `{"error":"URL not found"}` |

Every redirect sends `Cache-Control: private, no-cache`, ensuring clicks reach origin (webhooks fire, takedowns take effect promptly).

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/platform-redirect-dark.png">
  <img alt="Platform-specific redirect" src="docs/diagrams/platform-redirect.png" width="960">
</picture>

HTML behavior: desktop browsers immediately call `location.replace()` (no Back-button loop). On iOS, iPadOS (`MacIntel` + touch), and Android it attempts the deep link and falls back after 1.5 s if the page is still visible; hiding/blurring cancels the timer. A visible **Continue** link and `<noscript>` fallback are always present.

### Short keys

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/short-key-encoding-dark.png">
  <img alt="Short key encoding" src="docs/diagrams/short-key-encoding.png" width="960">
</picture>

- Format: current `[2 random][base62(id)][2 random]` (length `5..=15`) plus legacy `[1 random][base62(id)][1 random]` (length `3..=4`), alphabet `0-9A-Za-z`, ID part never starts with `'0'`
- Parsed on a stack buffer before any cache or database call; malformed paths immediately return `404`
- Random bytes are verified against the stored `random_key` to prevent sequential enumeration

### Creation: `POST /v1/urls`

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/url-creation-flow-dark.png">
  <img alt="URL creation flow" src="docs/diagrams/url-creation-flow.png" width="960">
</picture>

Browser mutations pass W3C Fetch Metadata (`Sec-Fetch-Site`/`Origin`) verification and per-IP GCRA rate limiting. Fields are trimmed/normalized once and hashed with length-prefixed SHA-256 across all 9 fields: `SELECT` → `INSERT … ON CONFLICT DO NOTHING` → fallback `SELECT` on concurrent race. Duplicates burn zero sequence values, and new links warm L1 immediately.

### Caching & horizontal scaling

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/diagrams/caching-strategy-dark.png">
  <img alt="Caching strategy" src="docs/diagrams/caching-strategy.png" width="960">
</picture>

An in-process `moka` TinyLFU cache keyed by decoded `i64 id` holds active links for 60 s and negative misses for 15 s under a 32 MiB byte+overhead cap, coalescing concurrent misses via singleflight. With no Redis L2 tier, each scaled replica caches its own hot working set independently while misses resolve in ~0.03 ms on `urls_pkey`.

## Quickstart

Requires Rust (1.88+, see `rust-version` in `Cargo.toml`) and Docker.

```bash
git clone https://github.com/lee-lou2/url-shortener.git && cd url-shortener
docker compose up -d      # PostgreSQL 15 for development
cp .env.example .env
cargo run                 # applies migrations, then http://localhost:3000
```

Checks: `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked` (integration tests use the compose PostgreSQL service).
Production runs behind a Cloudflare Tunnel (`Dockerfile`, `deploy.sh`) with `RUST_ENV=production`.

## Configuration

Authoritative, commented template: [`.env.example`](.env.example).

| Variable | Default | Description |
|----------|---------|-------------|
| `SERVER_PORT` | `3000` | HTTP port |
| `RUST_ENV` | `development` | `production`/`prod` or `development`/`dev`/`test` (case-insensitive) |
| `RUST_LOG` | `info` | `tracing_subscriber` log filter |
| `DB_HOST`, `DB_PORT`, `DB_USER`, `DB_PASSWORD`, `DB_NAME` | see `.env.example` | PostgreSQL connection parts (structured `PgConnectOptions`, special chars safe) |
| `DB_MAX_CONNECTIONS` | `10` | Pool size |
| `RUN_MIGRATIONS` | `true` | Apply `migrations/` on boot (`true`/`1`/`yes`, `false`/`0`/`no`) |
| `CACHE_MAX_CAPACITY_BYTES` | `33554432` | In-process L1 byte cap (32 MiB) |
| `RATE_LIMIT_PER_SECOND` | `10` | Per-IP rate for `POST /v1/urls` |
| `RATE_LIMIT_BURST_SIZE` | `20` | Per-IP burst for `POST /v1/urls` |
| `CORS_ORIGINS` | — | Comma-separated origins allowed for cross-origin browser `POST`s; empty allows same-origin browsers + direct API clients only (`*` is dev-only) |
| `SENTRY_DSN`, `SENTRY_TRACES_SAMPLE_RATE` | — / `0.1` | Optional 5xx error reporting and sample rate (`0.0..=1.0`) |

Removed legacy variables (`REDIS_*`, `JWT_*`, `CACHE_TTL_SECS`, etc.) are safely ignored, and empty values (`KEY=`) fall back to defaults.

## API

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Dark glassmorphism web UI (stateless, zero CDN, SHA-256 CSP) |
| `POST` | `/v1/urls` | Create or reuse a short link (public, Fetch Metadata + IP rate-limited) |
| `GET` | `/{short_key}` | `302` or HTML redirect (`HEAD` supported) |
| `GET` | `/health` | `{"status":"ok","version":"…"}` |
| `GET` | `/ready` | PostgreSQL `SELECT 1` probe (`200 ok` / `503 error`) |

### `POST /v1/urls`

Create a link in a single stateless request (no cookie or token required):

```bash
curl -s https://u.lou2.kr/v1/urls \
  -H 'Content-Type: application/json' \
  -d '{"defaultFallbackUrl":"https://example.com","iosDeepLink":"myapp://home","iosFallbackUrl":"https://apps.apple.com/app/myapp"}'
# => {"message":"URL shortened successfully","short_key":"Ab3D7Xy","created":true}
```

| Field | Required | Rules |
|-------|:--------:|-------|
| `defaultFallbackUrl` | ✓ | `http`/`https` URL, ≤ 2048 bytes |
| `iosFallbackUrl`, `androidFallbackUrl`, `ogImageUrl` | | `http`/`https` URL |
| `iosDeepLink`, `androidDeepLink` | | Any scheme except `javascript`, `data`, `vbscript`, `file`, `blob`, `about` |
| `ogTitle` / `ogDescription` | | ≤ 200 / ≤ 500 characters |
| `webhookUrl` | | Public-host `http`/`https` URL; receives `POST {"short_key","user_agent"}` per visit |

Empty strings count as absent. Success: `201` (`created: true`) or `200` (`created: false`). Every error is `{"error":"…"}`: `400`/`422` validation, `403` cross-origin rejected, `408` timeout (15 s), `413` body > 32 KiB, `429` rate limited (with `Retry-After`), `500` generic.

## Security model

- **XSS & clickjacking:** URL allow-lists (`http`/`https`; executable deep-link schemes denied) are enforced on creation and re-applied when compiling cached entries. Both views contain one bare `<script>` pinned by startup-computed `script-src 'sha256-…'` + `frame-ancestors 'none'`, pass dynamic data only via escaped `data-*` attributes, and send `X-Content-Type-Options: nosniff`.
- **Stateless CSRF defense:** W3C Fetch Metadata (`Sec-Fetch-Site`) + `Origin`/`Host` checks reject untrusted cross-site browser `POST /v1/urls` requests with `403` without cookies or tokens.
- **SSRF defense:** webhooks share one `reqwest::Client` with `SafeDnsResolver` rejecting loopback, RFC 1918, link-local/metadata (`169.254.169.254`), CGNAT, ULA, and IPv4-mapped IPv6 addresses, pinning validated IPs directly, disabling redirects, capping concurrency at 64, and draining for up to 5 s on shutdown.
- **Abuse limits:** per-IP GCRA rate limiting on `POST /v1/urls` (trusting `CF-Connecting-IP` only from loopback/private TCP peers, with 60 s idle-key pruning). Reads and probes are unthrottled.
- **Takedown:** `UPDATE urls SET deleted_at = now() WHERE id = …` (L1 entries expire within 60 s).

## Performance

Measured on a single worker thread (`TOKIO_WORKER_THREADS=1`) on local macOS (Apple Silicon) against PostgreSQL 15.19 (**1,000,000 rows**, ~85% plain + ~15% OG/deep-link) using the same compiled Rust HTTP/1.1 keep-alive harness before (`main` + Redis) and after (L1 `moka` + PostgreSQL); absolute throughput on a 1 CPU / 512 MB Linux container will differ.

| Experiment | Before (`main`) | After (L1 + PG) | Improvement |
|------------|----------------:|----------------:|-------------|
| PostgreSQL indexes (1M rows) | 5 indexes (**129.8 MB**) | 2 indexes (**78.0 MB**) | **-39.9%** (redundant dropped) |
| 10k bulk `INSERT` / WAL | 63.5k rows/s · 4.10 MB | **119.8k rows/s · 1.76 MB** | **1.89× · -57.1% WAL** |
| Hot Zipfian redirects (c=32, `br,gzip`) | 20,643 RPS (p99 2.27 ms) | **147,417 RPS** (p99 **0.31 ms**) | **7.14× RPS · 7.3× lower p99** |
| Cold random 1M (c=32, `br,gzip`) | 15,337 RPS (p99 3.17 ms) | **35,475 RPS** (p99 **1.32 ms**) | **2.31× RPS · 2.4× lower p99** |
| 2,000 wrong `random_key` (`id=42`) | **2,000** PG `idx_scan`s | **1** PG `idx_scan` | **-99.95% DB load** |
| 2,000 leading-zero aliases | `200` × 2,000 · cache polluted | **`404` × 2,000 · 0 DB hits** | 100% blocked before I/O |
| 200-way cold stampede | **20** PG scans (p99 16.80 ms) | **1** PG scan (p99 **6.76 ms**) | 20× fewer DB queries |
| 50× duplicate create seq burn / OG hijack | 50 IDs burned · hijacked | **0 IDs burned · distinct keys** | 9-field SHA-256 + `SELECT`-first |

## Project layout

```
├── src/          11 flat modules (main, lib, config, error, short_key, guard, links, redirect, cache, webhook, handlers)
├── views/        index.html (dark glassmorphism UI), redirect.html (Askama template for deep-link / OG redirects)
├── migrations/   SQLx migrations (never edit applied files)
├── tests/        router integration tests against real PostgreSQL
├── compose.yaml  PostgreSQL 15 for development
└── docs/         screenshot and editorial diagrams (docs/diagrams/README.md)
```

Contributors and AI coding agents: read [AGENTS.md](AGENTS.md).

## License

MIT License
