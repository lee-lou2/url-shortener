use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Router,
};
use jsonwebtoken::{
    decode, encode, get_current_timestamp, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::KeyExtractor, GovernorError, GovernorLayer,
};

use crate::{
    config::Config,
    error::{json_error, AppError, AppResult},
    handlers::AppState,
};

pub struct Auth {
    encoding: EncodingKey,
    decoding: DecodingKey,
    validation: Validation,
    ttl: Duration,
    secure: bool,
}

#[derive(Deserialize, Serialize)]
struct Claims {
    sub: String,
    exp: u64,
    iat: u64,
}

impl Auth {
    pub fn new(secret: &[u8], ttl: Duration, secure: bool) -> Self {
        let mut validation = Validation::default();
        validation.leeway = 0;
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
            validation,
            ttl,
            secure,
        }
    }

    pub fn guest_cookie(&self) -> AppResult<String> {
        let now = get_current_timestamp();
        let claims = Claims {
            sub: "guest".into(),
            iat: now,
            exp: now + self.ttl.as_secs(),
        };
        let token = encode(&Header::default(), &claims, &self.encoding)
            .map_err(|_| AppError::Internal("Unable to sign guest token"))?;
        let secure = if self.secure { "; Secure" } else { "" };
        Ok(format!(
            "token={token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{secure}",
            self.ttl.as_secs(),
        ))
    }

    fn valid(&self, token: &str) -> bool {
        decode::<Claims>(token, &self.decoding, &self.validation).is_ok()
    }
}

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    if let Some((scheme, token)) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
    {
        if scheme.eq_ignore_ascii_case("Bearer") {
            return Some(token.trim_start_matches(' '));
        }
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            value
                .split(';')
                .filter_map(|pair| pair.trim().split_once('='))
                .find_map(|(name, token)| (name == "token").then_some(token))
        })
}

pub(crate) async fn authenticate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> AppResult<Response> {
    if !extract_token(request.headers()).is_some_and(|token| state.auth.valid(token)) {
        return Err(AppError::Request(
            StatusCode::UNAUTHORIZED,
            "Invalid or missing token".into(),
        ));
    }
    Ok(next.run(request).await)
}

pub(crate) fn limit_writes(
    routes: Router<AppState>,
    config: &Config,
    shutdown: CancellationToken,
) -> AppResult<Router<AppState>> {
    let period = Duration::from_secs(1)
        .checked_div(config.rate_limit_per_second)
        .ok_or(AppError::Internal("Invalid rate limiter rate"))?;
    let limiter = GovernorConfigBuilder::default()
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
        let trusted = match peer {
            IpAddr::V4(peer) => peer.is_loopback() || peer.is_private(),
            IpAddr::V6(peer) => peer.is_loopback() || peer.is_unique_local(),
        };
        let mut headers = request.headers().get_all("cf-connecting-ip").iter();
        if trusted {
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

    #[test]
    fn issues_host_only_cookie_with_ttl() {
        let auth = Auth::new(b"shared test secret", Duration::from_secs(60), true);
        let cookie = auth.guest_cookie().unwrap();
        let (token, settings) = cookie
            .strip_prefix("token=")
            .unwrap()
            .split_once(';')
            .unwrap();
        assert_eq!(
            settings,
            " HttpOnly; SameSite=Lax; Path=/; Max-Age=60; Secure"
        );
        assert!(auth.valid(token));
        assert!(!Auth::new(b"other test secret", Duration::from_secs(60), false).valid(token));
    }

    #[test]
    fn extracts_cookie_and_case_insensitive_bearer() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, HeaderValue::from_static("session=x"));
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("x=1; token=second"),
        );
        assert_eq!(extract_token(&headers), Some("second"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("bEaReR  first"),
        );
        assert_eq!(extract_token(&headers), Some("first"));
    }

    #[test]
    fn rejects_expired_token() {
        let auth = Auth::new(b"shared test secret", Duration::from_secs(60), false);
        let claims = Claims {
            sub: "guest".into(),
            exp: get_current_timestamp() - 1,
            iat: 0,
        };
        let token = encode(&Header::default(), &claims, &auth.encoding).unwrap();
        assert!(!auth.valid(&token));
    }
}
