# AGENTS.md

Single source of truth for AI coding agents and human contributors (`CLAUDE.md` only imports this file).
Rule for this file: never restate what code, `.env.example` or `migrations/` already say — link to them.

**What:** Rust (Axum 0.8, SQLx, PostgreSQL) URL shortener, live at `https://u.lou2.kr` behind a Cloudflare Tunnel
(`http://url-shortener:3000`, 1 CPU / 512 MB). Production rows and issued short keys are live, so the invariants below are not negotiable.

## Architecture
11 flat `src/` files, no sub-directories, no `static`/`Lazy` globals (all state lives in `AppState`).
`GET /{short_key}` runs `parse_short_key` (before any I/O), then one L1 `moka` cache keyed by `i64 id`
(found 60 s / missing 15 s, byte-capped, singleflight), then optional L2 Redis (`ConnectionManager`, `u:{id}` JSON, circuit breaker;
L1 + PostgreSQL only when `REDIS_HOST` is unset or Redis is down), then PostgreSQL. A link compiles into one of two `RedirectPayload`s:
no deep links / platform fallbacks / OG fields → `302` + `Location`; otherwise → pre-rendered `200` HTML (`views/redirect.html`).
Short key = `prefix(2) + base62(id) + suffix(2)` (v1). Dedupe = SHA-256 over 9 normalized fields.

## Module map (`src/`)
| File | Single responsibility |
|---|---|
| `main.rs` | Binary only: tracing, Sentry, `Config`, connect, migrate, serve, graceful shutdown (imports `url_shortener::*`) |
| `lib.rs` | Library root: declares the 9 crate modules imported by `main.rs` and `tests/` |
| `config.rs` | Env → `Config`, fail-fast validation, `PgConnectOptions` / Redis `ConnectionInfo` |
| `error.rs` | `AppError`, `ApiJson<T>`: every error is `{"error": "..."}`; 5xx messages are generic |
| `short_key.rs` | Inline base62, `parse_short_key`, `encode_short_key`, golden vectors |
| `auth.rs` | Guest JWT mint/verify, hand-rolled cookie, `Bearer`/cookie middleware, `CF-Connecting-IP` write rate limiter |
| `links.rs` | Normalize + validate request, 9-field SHA-256 hash, SQL (`find_active`, `create_or_find`) |
| `redirect.rs` | `LinkRecord` → `RedirectPayload`, `RedirectPage` template, CSP script hashes |
| `cache.rs` | L1 moka + optional L2 Redis (60 s TTL, timeouts, breaker, async write-back) |
| `webhook.rs` | Fire-and-forget POST, `SafeDnsResolver`, bounded concurrency, drain on shutdown |
| `handlers.rs` | `AppState`, `create_routes()`, middleware (body limit, 15 s timeout, CORS, headers), thin handlers (`index`, `create_link`, `redirect`, `health`, `ready`) |

Other: `views/index.html` (served verbatim), `views/redirect.html` (Askama `RedirectPage`), `migrations/`, `tests/`,
`compose.yaml`, `.env.example`, `docs/diagrams/` (`src/*.html` are the sources of the PNGs).

## Invariants (never break)
1. **Migrations:** never edit an applied file (SQLx checksums). Schema change = new migration;
   `CONCURRENTLY` = one statement per `-- no-transaction` file.
2. **Short key:** length `5..=15`, ASCII alphanumeric, alphabet `0-9A-Za-z`, id part never starts with `'0'`, `1 <= id <= i64::MAX`,
   parsed before any cache/DB call. Anything invalid → `404 {"error":"URL not found"}` (also `/favicon.ico`, `/robots.txt`).
3. **Contract:** `GET /`, `POST /v1/urls`, `GET /{short_key}`, `GET /health`, `GET /ready`. Request fields are camelCase;
   response is `{message, short_key, created}` (`201` new, `200` existing); errors are `{"error": "..."}`.
   Env var names are the production contract: add, never rename or remove.
4. **URLs:** destination, platform fallbacks, OG image and webhook are `http`/`https` only (≤ 2048 bytes, no control bytes,
   stored as `Url::as_str()`). Deep links may use any scheme except `javascript data vbscript file blob about`.
   Re-validate when compiling a cache entry (legacy rows can predate these rules).
5. **Webhooks:** only through the shared client with `SafeDnsResolver`, redirects off, literal-IP hosts checked at create **and** dispatch.
6. **Views/CSP:** each view has exactly one bare `<script>` (no attributes, no template interpolation inside), one `<style>`,
   no inline `on*=` / `style=`, no external resources. Dynamic data reaches JS only through `data-*` attributes.
   The server hashes the script text at startup for `script-src 'sha256-…'`, so scripts are edited freely but must stay a single block.
7. **SQL:** parameterized binds only (never `format!`); always filter `deleted_at IS NULL AND is_active = true`.
   Takedown = `UPDATE urls SET deleted_at = now() WHERE id = $1` (L1 expires within 60 s; also `DEL u:{id}` if L2 is on).
8. **Code:** zero global statics; no `.unwrap()` / `.expect()` / `panic!` outside tests and `main` startup; no `println!` (use `tracing`);
   config only via `Config` in `AppState`; no file over ~300 lines (split by responsibility).
9. **Rate limit:** only `POST /v1/urls` and `GET /`; trust `CF-Connecting-IP` only when the TCP peer is loopback/private.
10. **Docs:** no performance number without a measured benchmark. Change the contract, env vars, module map or an invariant
    ⇒ update this file and both READMEs in the same PR.

## Decisions already made (do not relitigate without measurements)
- Plain links `302`, everything else HTML. UA-based server `302` was rejected: the server cannot see iPadOS (desktop-class UA).
- L2 Redis is optional; L1 + PostgreSQL is a supported mode. No compression layer (Cloudflare compresses at the edge); CORS off unless `CORS_ORIGINS` is set.
- The guest JWT is a speed bump, not an identity. Legacy 32-char `xxh3` hashes coexist with new 64-char SHA-256 hashes (no backfill).

## Frontend (`views/`)
No build step, no JS toolchain, system fonts, light/dark through `prefers-color-scheme`. Keep both pages accessible (labels, focus rings,
`aria-live`, contrast) and free of `innerHTML`: user text goes through `textContent` only. `redirect.html` must keep `location.replace`,
iPadOS detection, the cancel-on-`visibilitychange`/`pagehide`/`blur` timer and the visible "Continue" link.

## Commands
```bash
docker compose up -d && cp .env.example .env   # Postgres 15 + Redis 7 for dev/tests (first run: copy env)
cargo run                                      # applies migrations on boot (RUN_MIGRATIONS=true) → http://localhost:3000
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                                     # router tests use real Postgres/Redis: keep compose up
```
CI (`.github/workflows/ci.yml`) runs exactly these three checks with `postgres:15-alpine` and `redis:7-alpine`. Run them before every push.

## Adding an endpoint
1. Request type + normalization/validation in `links.rs` (new file only for a new responsibility).
2. Thin handler in `handlers.rs` returning `Result<_, AppError>`; no `unwrap`.
3. Route, body limit, rate-limit and auth layers in `handlers.rs::create_routes()`.
4. Tests in `tests/` (HTTP status, JSON error shape, DB side effects); update the README API table.

## Git & PRs
Branch from `main`, open small PRs against `main`. Conventional Commits: `feat|fix|refactor|perf|docs|test|ci|chore(scope): subject`.
PR body: what changed, why, how verified (paste the three commands' result). Never commit `.env`, secrets or `target/`; never force-push `main`.
