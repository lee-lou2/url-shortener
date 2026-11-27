# AGENTS.md

Single source of truth for AI coding agents and human contributors (`CLAUDE.md` only imports this file).
Rule: never restate what code, `.env.example`, or `migrations/` already say — link to them.

**What:** Stateless Rust (Axum 0.8 + PostgreSQL + in-process `moka` TinyLFU cache) public URL shortener, live at `https://u.lou2.kr` behind a Cloudflare Tunnel (`http://url-shortener:3000`, 1 CPU / 512 MB). Production rows and issued short keys are live; invariants below are non-negotiable.

## Architecture
11 flat `src/` files, zero subdirectories, zero mutable global statics (all state lives in `AppState`; only `#[global_allocator]` mimalloc in `main.rs`).
- **`GET /{short_key}`**: `parse_short_key` runs before any I/O → in-process `moka` L1 cache keyed by `i64 id` (Found 60 s / Missing 15 s, 32 MiB byte-capped, `try_get_with` singleflight) → PostgreSQL `urls_pkey` lookup on miss. Compiles to one of two `RedirectPayload`s: plain destination → `302 Found` + `Location`; any deep-link / platform fallback / OG field → pre-rendered `200 OK` HTML (`views/redirect.html`).
- **`POST /v1/urls`**: Public stateless endpoint protected by W3C Fetch Metadata (`Sec-Fetch-Site` / `Origin`) cross-origin guard + per-IP GCRA rate limit (`CF-Connecting-IP` trusted only from private/loopback peers). Normalizes & validates once, computes 9-field length-prefixed SHA-256, runs `SELECT` → `INSERT … ON CONFLICT DO NOTHING` (0 sequence burn on duplicates), and warms L1 on create.
- **Horizontal scaling**: Completely stateless across replicas (no Redis L2, no session cookies, no `JWT_SECRET`). Each replica's in-process TinyLFU L1 holds the hot working set independently; misses resolve in ~0.03 ms via `urls_pkey`.

## Module map (`src/`)
| File | Single responsibility |
|---|---|
| `main.rs` | Binary only: mimalloc, tracing, Sentry, `Config`, Postgres pool, migrations, serve, graceful shutdown |
| `lib.rs` | Library root declaring the 9 modules imported by `main.rs` and `tests/` |
| `config.rs` | Env → `Config`, fail-fast validation, structured `PgConnectOptions` |
| `error.rs` | `AppError`, `ApiJson<T>`: every error is `{"error": "..."}`; 5xx messages are generic |
| `short_key.rs` | Zero-alloc inline Base62, `RandomKey` (v1 4-char + legacy 2-char), `parse_short_key`, `encode_short_key` |
| `guard.rs` | W3C Fetch Metadata / `Origin` cross-origin guard + `CF-Connecting-IP` GCRA write rate limiter |
| `links.rs` | Single-pass URL/deep-link allow-list validation, 9-field SHA-256, SQL (`find_active`, `create_or_find`) |
| `redirect.rs` | `LinkRecord` → `RedirectPayload` (`302` or HTML `Bytes`), Askama `RedirectPage`, SHA-256 CSP hashes |
| `cache.rs` | In-process `moka` TinyLFU cache (`i64 id` key, Found 60 s / Missing 15 s, singleflight, `warm`) |
| `webhook.rs` | Shared `reqwest::Client`, `SafeDnsResolver` SSRF guard, bounded concurrency (64), 5 s shutdown drain |
| `handlers.rs` | `AppState`, `create_routes()`, HTTP policy (32 KiB body, 15 s timeout, `nosniff`, CORS), thin handlers |

Other: `views/index.html` (static dark glassmorphism UI), `views/redirect.html` (Askama `RedirectPage`), `migrations/`, `tests/api.rs`, `compose.yaml`, `.env.example`, `docs/diagrams/`.

## Invariants (never break)
1. **Migrations:** never edit an applied migration (SQLx checksums). Schema changes = new migration; `CONCURRENTLY` = one statement per `-- no-transaction` file.
2. **Short key:** current v1 length `5..=15` (`prefix(2)+base62(id)+suffix(2)`) plus legacy 2-char `random_key` length `3..=4` (`prefix(1)+base62(id)+suffix(1)`), ASCII alphanumeric `0-9A-Za-z`, ID part never starts with `'0'`, `1 <= id <= i64::MAX`, parsed before any cache/DB call. Invalid → `404 {"error":"URL not found"}`.
3. **Contract:** `GET /`, `POST /v1/urls`, `GET /{short_key}`, `GET /health`, `GET /ready`. Request fields camelCase; create response `{message, short_key, created}` (`201` new / `200` existing); errors `{"error": "..."}`. Removed legacy env vars (`REDIS_*`, `JWT_*`, etc.) are safely ignored; never reuse an old name with new meaning.
4. **URLs:** destination, fallbacks, OG image, and webhook are `http`/`https` only (≤ 2048 bytes, no control bytes, stored as `Url::as_str()`). Deep links allow any scheme except `javascript data vbscript file blob about`. Re-validate when compiling a cache entry (defends against legacy rows).
5. **Webhooks:** only via the shared client with `SafeDnsResolver`, redirects disabled (`Policy::none()`), literal-IP hosts checked at create **and** dispatch.
6. **Views/CSP:** each view has exactly one bare `<script>` (no attributes, no template interpolation inside), one `<style>`, no inline `on*=`/`style=`, zero external CDN resources. Dynamic data reaches JS only via `data-*` attributes. Startup computes `script-src 'sha256-…'` + `frame-ancestors 'none'`.
7. **SQL:** parameterized binds only (never `format!`); always filter `deleted_at IS NULL AND is_active = true`. Takedown = `UPDATE urls SET deleted_at = now() WHERE id = $1` (L1 expires within 60 s).
8. **Code:** zero mutable global statics; no `.unwrap()`/`.expect()`/`panic!` outside tests and `main` startup; no `println!` (use `tracing`); keep every file in `src/` under ~300 lines.
9. **Abuse guards:** `POST /v1/urls` is guarded by `OriginGuard` (`Sec-Fetch-Site`/`Origin`) + per-IP GCRA rate limit (`CF-Connecting-IP` trusted only from loopback/private TCP peers). `GET /`, redirects, and probes are unthrottled.
10. **Docs:** no performance claim without measured data. Contract, env, module map, or invariant changes must update this file and both READMEs in the same PR.

## Decisions already made (do not relitigate without measurements)
- **No Redis L2:** single-CPU L1-only (`moka` + PG `urls_pkey`) measured 22–25 % faster on both hot (147k RPS) and cold (35k RPS) traffic without long-tail L2 overhead or Redis SPOF. Each scaled replica holds its own hot working set cleanly.
- **No Guest JWT:** public service; stateless W3C Fetch Metadata (`Sec-Fetch-Site`/`Origin`) + GCRA rate limit replaces cookie/JWT ceremony with stronger CSRF protection and 1-call API ergonomics.
- **2-mode redirect (`302` vs HTML):** server-side UA sniffing cannot detect iPadOS 13+ (`Macintosh` UA). No compression layer (Cloudflare compresses at edge); CORS off unless `CORS_ORIGINS` is set.

## Frontend (`views/`)
No build step, no JS toolchain, system fonts, zero external CDN (dark glassmorphism CSS). Keep both pages accessible (labels, focus rings, `aria-live`, `prefers-reduced-motion`) and free of `innerHTML` (`textContent` only). `redirect.html` must keep `location.replace`, iPadOS detection, cancel-on-`visibilitychange`/`pagehide`/`blur`, and visible **Continue** link.

## Commands
```bash
docker compose up -d && cp .env.example .env   # PostgreSQL 15 for dev/tests
cargo run                                      # applies migrations on boot → http://localhost:3000
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked                            # router tests run against real PostgreSQL
```
CI (`.github/workflows/ci.yml`) runs these checks plus `cargo check --locked --all-targets` on Rust 1.88 (`Dockerfile` MSRV).

## Git & PRs
Branch from `main`, open focused PRs against `main` using Conventional Commits. Never commit `.env`, secrets, or `target/`; never force-push `main`.
