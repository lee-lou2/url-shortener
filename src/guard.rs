//! Request-abuse guards for public endpoints:
//!
//! 1. **W3C Fetch Metadata + `Origin` cross-origin protection** (stateless CSRF
//!    defense with zero cookies or tokens): blocks cross-site browser mutations
//!    unless the origin is explicitly allow-listed in `CORS_ORIGINS`, while
//!    allowing same-origin browsers and non-browser API clients (`curl`, SDKs).
//! 2. **Per-IP GCRA write rate limiter** (`tower_governor`): trusts
//!    `CF-Connecting-IP` only when the direct TCP peer is loopback/private,
//!    returns JSON `429` + `Retry-After`, and prunes idle keys every 60 s.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, uri::Authority, HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Router,
};
use tokio_util::sync::CancellationToken;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::KeyExtractor, GovernorError, GovernorLayer,
};

use crate::{
    config::Config,
    error::{json_error, AppError, AppResult},
    handlers::AppState,
};

/// Stateless cross-origin request policy derived from `CORS_ORIGINS`.
pub struct OriginGuard {
    allow_any: bool,
    allowed_origins: Vec<String>,
}

impl OriginGuard {
    pub fn from_config(config: &Config) -> Self {
        let allow_any = config.cors_origins.iter().any(|origin| origin == "*");
        let allowed_origins = config
            .cors_origins
            .iter()
            .filter_map(|origin| origin.to_str().ok())
            .filter(|origin| *origin != "*")
            .map(str::to_owned)
            .collect();
        Self {
            allow_any,
            allowed_origins,
        }
    }

    fn is_allowed_origin(&self, origin: &str) -> bool {
        self.allow_any || self.allowed_origins.iter().any(|allowed| allowed == origin)
    }

    /// Evaluates W3C Fetch Metadata (`Sec-Fetch-Site`) with `Origin`/`Host` fallback.
    pub fn check(&self, method: &Method, headers: &HeaderMap, trusted_proxy: bool) -> bool {
        if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
            return true;
        }
        let Ok(origin) = single_header(headers, header::ORIGIN.as_str()) else {
            return false;
        };
        let Ok(site) = single_header(headers, "sec-fetch-site") else {
            return false;
        };

        if let Some(site) = site {
            if matches!(site, "same-origin" | "none") {
                return true;
            }
            return origin.is_some_and(|value| self.is_allowed_origin(value));
        }

        // No `Sec-Fetch-Site`: non-browser clients omit `Origin` and pass through;
        // older browsers that send `Origin` must match `Host` (and proxy scheme) or `CORS_ORIGINS`.
        let Some(origin) = origin else {
            return true;
        };
        if self.is_allowed_origin(origin) {
            return true;
        }
        origin_matches_host(origin, headers, trusted_proxy)
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(Some).map_err(|_| ())
}

fn origin_matches_host(origin: &str, headers: &HeaderMap, trusted_proxy: bool) -> bool {
    let Ok(Some(host)) = single_header(headers, header::HOST.as_str()) else {
        return false;
    };
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    let default_port = match url.scheme() {
        "http" => 80_u16,
        "https" => 443_u16,
        _ => return false,
    };
    if trusted_proxy {
        match single_header(headers, "x-forwarded-proto") {
            Ok(Some(proto)) if !proto.eq_ignore_ascii_case(url.scheme()) => return false,
            Err(()) => return false,
            _ => {}
        }
    }
    let Some(origin_host) = url.host_str() else {
        return false;
    };
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    if authority.as_str().contains('@') {
        return false;
    }
    let origin_port = url.port().unwrap_or(default_port);
    let host_port = authority.port_u16().unwrap_or(default_port);
    origin_host.eq_ignore_ascii_case(authority.host()) && origin_port == host_port
}

fn is_trusted_peer(peer: IpAddr) -> bool {
    match peer.to_canonical() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
    }
}

pub(crate) async fn verify_origin(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> AppResult<Response> {
    let trusted_proxy = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|ConnectInfo(peer)| is_trusted_peer(peer.ip()));
    if !state
        .origin
        .check(request.method(), request.headers(), trusted_proxy)
    {
        return Err(AppError::Request(
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected".into(),
        ));
    }
    Ok(next.run(request).await)
}

/// Applies per-IP GCRA rate limiting to `POST` write routes and spawns periodic state pruning.
pub(crate) fn limit_writes(
    routes: Router<AppState>,
    config: &Config,
    shutdown: CancellationToken,
) -> AppResult<Router<AppState>> {
    // `tower_governor::per_second(n)` sets a 1-token-per-n-seconds period, so we
    // compute the exact inter-token interval explicitly via `.period()`.
    let period = Duration::from_secs(1)
        .checked_div(config.rate_limit_per_second)
        .ok_or(AppError::Internal("Invalid rate limiter rate"))?;
    let limiter = GovernorConfigBuilder::default()
        .methods(vec![Method::POST])
        .period(period)
        .burst_size(config.rate_limit_burst_size)
        .key_extractor(ClientIp)
        .finish()
        .ok_or(AppError::Internal("Invalid rate limiter configuration"))?;

    let cleanup = limiter.limiter().clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep(Duration::from_secs(60)) => {
                    cleanup.retain_recent();
                    cleanup.shrink_to_fit();
                }
            }
        }
    });

    Ok(routes
        .route_layer(GovernorLayer::new(limiter).error_handler(|error| rate_limit_error(&error))))
}

#[derive(Clone, Copy)]
struct ClientIp;

impl KeyExtractor for ClientIp {
    type Key = IpAddr;

    fn extract<T>(&self, request: &axum::http::Request<T>) -> Result<IpAddr, GovernorError> {
        let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
            return Ok(Ipv4Addr::LOCALHOST.into());
        };
        let peer = peer.ip().to_canonical();
        let mut headers = request.headers().get_all("cf-connecting-ip").iter();
        if is_trusted_peer(peer) {
            if let (Some(value), None) = (headers.next(), headers.next()) {
                if let Some(client) = value
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse::<IpAddr>().ok())
                {
                    return Ok(client.to_canonical());
                }
            }
        }
        Ok(peer)
    }
}

fn rate_limit_error(error: &GovernorError) -> Response {
    let GovernorError::TooManyRequests { wait_time, .. } = error else {
        return AppError::Internal("Rate limiter failed").into_response();
    };
    let mut response = json_error(StatusCode::TOO_MANY_REQUESTS, "Too many requests");
    if let Ok(retry) = HeaderValue::from_str(&(*wait_time).max(1).to_string()) {
        response.headers_mut().insert(header::RETRY_AFTER, retry);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(origins: &[&str]) -> OriginGuard {
        OriginGuard {
            allow_any: origins.contains(&"*"),
            allowed_origins: origins
                .iter()
                .filter(|origin| **origin != "*")
                .map(|origin| (*origin).to_owned())
                .collect(),
        }
    }

    #[test]
    fn fetch_metadata_blocks_cross_site_and_allows_same_origin_and_api_clients() {
        let strict = guard(&[]);
        // Non-browser client (no Sec-Fetch-Site, no Origin) passes.
        assert!(strict.check(&Method::POST, &HeaderMap::new(), false));

        let mut same = HeaderMap::new();
        same.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(strict.check(&Method::POST, &same, false));

        let mut cross = HeaderMap::new();
        cross.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        cross.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.test"),
        );
        assert!(!strict.check(&Method::POST, &cross, false));

        // Explicit CORS allow-list permits trusted cross-site origin.
        let allowed = guard(&["https://app.example.com"]);
        let mut trusted = HeaderMap::new();
        trusted.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        trusted.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://app.example.com"),
        );
        assert!(allowed.check(&Method::POST, &trusted, false));
        assert!(!allowed.check(&Method::POST, &cross, false));
    }

    #[test]
    fn origin_fallback_checks_host_port_and_proxy_scheme_for_older_browsers() {
        let strict = guard(&[]);
        let mut matching = HeaderMap::new();
        matching.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://u.lou2.kr"),
        );
        matching.insert(header::HOST, HeaderValue::from_static("u.lou2.kr:443"));
        matching.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(strict.check(&Method::POST, &matching, true));

        // HTTP origin targeting an HTTPS proxy must be rejected.
        let mut downgrade = HeaderMap::new();
        downgrade.insert(header::ORIGIN, HeaderValue::from_static("http://u.lou2.kr"));
        downgrade.insert(header::HOST, HeaderValue::from_static("u.lou2.kr"));
        downgrade.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        assert!(!strict.check(&Method::POST, &downgrade, true));

        let mut mismatch = HeaderMap::new();
        mismatch.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://evil.test"),
        );
        mismatch.insert(header::HOST, HeaderValue::from_static("u.lou2.kr"));
        assert!(!strict.check(&Method::POST, &mismatch, false));

        let mut null_origin = HeaderMap::new();
        null_origin.insert(header::ORIGIN, HeaderValue::from_static("null"));
        null_origin.insert(header::HOST, HeaderValue::from_static("u.lou2.kr"));
        assert!(!strict.check(&Method::POST, &null_origin, false));

        let mut duplicate = HeaderMap::new();
        duplicate.append("sec-fetch-site", HeaderValue::from_static("same-origin"));
        duplicate.append("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(!strict.check(&Method::POST, &duplicate, false));
    }
}
