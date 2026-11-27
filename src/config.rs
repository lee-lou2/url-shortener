use std::{env, net::SocketAddr, time::Duration};

use axum::http::HeaderValue;
use redis::{ConnectionAddr, ConnectionInfo, RedisConnectionInfo};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid configuration for {0}")]
pub struct ConfigError(&'static str);

pub struct Config {
    pub bind: SocketAddr,
    pub production: bool,
    pub jwt_secret: String,
    pub jwt_ttl: Duration,
    pub postgres: PgConnectOptions,
    pub db_max_connections: u32,
    pub redis: Option<ConnectionInfo>,
    pub cache_max_capacity_bytes: u64,
    pub rate_limit_per_second: u32,
    pub rate_limit_burst_size: u32,
    pub cors_origins: Vec<HeaderValue>,
    pub run_migrations: bool,
    pub sentry_dsn: String,
    pub sentry_traces_sample_rate: f32,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let production = match lookup("RUST_ENV").as_deref() {
            Some("production" | "prod") => true,
            Some("development" | "dev" | "test") | None => false,
            Some(_) => return Err(ConfigError("RUST_ENV")),
        };
        let jwt_secret = match lookup("JWT_SECRET").filter(|secret| !secret.trim().is_empty()) {
            Some(secret) if !production || secret.len() >= 32 => secret,
            None if !production => "local-development-url-shortener-secret".to_owned(),
            _ => return Err(ConfigError("JWT_SECRET")),
        };
        let jwt_hours: u64 = parse(&lookup, "JWT_EXPIRATION_HOURS", 24)?;
        if !(1..=8760).contains(&jwt_hours) {
            return Err(ConfigError("JWT_EXPIRATION_HOURS"));
        }
        let server_port = parse(&lookup, "SERVER_PORT", 3000_u16)?;
        let db_max_connections = parse(&lookup, "DB_MAX_CONNECTIONS", 10_u32)?;
        let rate_limit_per_second = parse(&lookup, "RATE_LIMIT_PER_SECOND", 10_u32)?;
        let rate_limit_burst_size = parse(&lookup, "RATE_LIMIT_BURST_SIZE", 20_u32)?;
        for (value, name) in [
            (u32::from(server_port), "SERVER_PORT"),
            (db_max_connections, "DB_MAX_CONNECTIONS"),
            (rate_limit_per_second, "RATE_LIMIT_PER_SECOND"),
            (rate_limit_burst_size, "RATE_LIMIT_BURST_SIZE"),
        ] {
            if value == 0 || (name == "RATE_LIMIT_PER_SECOND" && value > 1_000_000_000) {
                return Err(ConfigError(name));
            }
        }
        let cors_origins = lookup("CORS_ORIGINS")
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                value
                    .split(',')
                    .map(|origin| cors_origin(origin.trim(), production))
                    .collect()
            })
            .transpose()?
            .unwrap_or_default();

        let postgres = PgConnectOptions::new()
            .host(&lookup("DB_HOST").unwrap_or_else(|| "localhost".into()))
            .port(parse(&lookup, "DB_PORT", 5432)?)
            .username(&lookup("DB_USER").unwrap_or_else(|| "postgres".into()))
            .password(&lookup("DB_PASSWORD").unwrap_or_else(|| "postgres".into()))
            .database(&lookup("DB_NAME").unwrap_or_else(|| "postgres".into()));
        let redis = lookup("REDIS_HOST")
            .filter(|host| !host.is_empty())
            .map(|host| {
                let db = parse(&lookup, "REDIS_DB", 0_i64)?;
                if db < 0 {
                    return Err(ConfigError("REDIS_DB"));
                }
                Ok(ConnectionInfo {
                    addr: ConnectionAddr::Tcp(host, parse(&lookup, "REDIS_PORT", 6379)?),
                    redis: RedisConnectionInfo {
                        db,
                        username: lookup("REDIS_USER").filter(|value| !value.is_empty()),
                        password: lookup("REDIS_PASSWORD").filter(|value| !value.is_empty()),
                        ..RedisConnectionInfo::default()
                    },
                })
            })
            .transpose()?;
        let sample_rate = parse(&lookup, "SENTRY_TRACES_SAMPLE_RATE", 0.1_f32)?;
        if !(0.0..=1.0).contains(&sample_rate) {
            return Err(ConfigError("SENTRY_TRACES_SAMPLE_RATE"));
        }

        Ok(Self {
            bind: SocketAddr::from(([0, 0, 0, 0], server_port)),
            production,
            jwt_secret,
            jwt_ttl: Duration::from_secs(jwt_hours * 3600),
            postgres,
            db_max_connections,
            redis,
            cache_max_capacity_bytes: parse(&lookup, "CACHE_MAX_CAPACITY_BYTES", 32 * 1024 * 1024)?,
            cors_origins,
            rate_limit_per_second,
            rate_limit_burst_size,
            run_migrations: parse(&lookup, "RUN_MIGRATIONS", true)?,
            sentry_dsn: lookup("SENTRY_DSN").unwrap_or_default(),
            sentry_traces_sample_rate: sample_rate,
        })
    }

    pub async fn connect_db(&self) -> Result<PgPool, sqlx::Error> {
        PgPoolOptions::new()
            .max_connections(self.db_max_connections)
            .acquire_timeout(Duration::from_secs(3))
            .test_before_acquire(true)
            .connect_with(self.postgres.clone())
            .await
    }
}

fn parse<T: std::str::FromStr>(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError> {
    lookup(name).map_or(Ok(default), |value| {
        value.parse().map_err(|_| ConfigError(name))
    })
}

fn cors_origin(origin: &str, production: bool) -> Result<HeaderValue, ConfigError> {
    if origin == "*" && !production {
        return Ok(HeaderValue::from_static("*"));
    }
    let url = url::Url::parse(origin).map_err(|_| ConfigError("CORS_ORIGINS"))?;
    if !matches!(url.scheme(), "http" | "https") || origin != url.origin().ascii_serialization() {
        return Err(ConfigError("CORS_ORIGINS"));
    }
    HeaderValue::from_str(origin).map_err(|_| ConfigError("CORS_ORIGINS"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(values: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(|key| {
            values
                .iter()
                .find(|entry| entry.0 == key)
                .map(|entry| entry.1.into())
        })
    }

    #[test]
    fn requires_production_secret() {
        assert!(config(&[("RUST_ENV", "production")]).is_err());
        assert!(config(&[("RUST_ENV", "production"), ("JWT_SECRET", "short")]).is_err());
        assert!(config(&[("RUST_ENV", "production"), ("JWT_SECRET", &"x".repeat(32))]).is_ok());
    }

    #[test]
    fn validates_ttl_and_rate_configuration() {
        for (name, value) in [
            ("JWT_EXPIRATION_HOURS", "0"),
            ("JWT_EXPIRATION_HOURS", "8761"),
            ("SERVER_PORT", "abc"),
            ("RATE_LIMIT_BURST_SIZE", "0"),
        ] {
            assert!(config(&[(name, value)]).is_err());
        }
    }

    #[test]
    fn uses_structured_redis_credentials() {
        let config = config(&[
            ("REDIS_HOST", "localhost"),
            ("REDIS_DB", "14"),
            ("REDIS_PASSWORD", "@:/#?%"),
        ])
        .unwrap();
        let redis = config.redis.unwrap();
        assert_eq!(redis.redis.db, 14);
        assert_eq!(redis.redis.password.as_deref(), Some("@:/#?%"));
    }
}
