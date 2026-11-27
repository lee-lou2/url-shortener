//! Typed environment configuration with fail-fast validation at startup.
//!
//! Credentials are passed through structured [`PgConnectOptions`] rather than
//! interpolated into URL strings, so special characters in passwords are safe.

use std::{env, net::SocketAddr, time::Duration};

use axum::http::HeaderValue;
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("Invalid configuration for {0}")]
pub struct ConfigError(&'static str);

/// Runtime configuration parsed once from environment variables at boot.
pub struct Config {
    pub bind: SocketAddr,
    pub production: bool,
    pub postgres: PgConnectOptions,
    pub db_max_connections: u32,
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
        let production = match lookup("RUST_ENV")
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("production" | "prod") => true,
            Some("development" | "dev" | "test") | None => false,
            Some(_) => return Err(ConfigError("RUST_ENV")),
        };

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
                let mut origins = Vec::new();
                for origin in value
                    .split(',')
                    .map(str::trim)
                    .filter(|origin| !origin.is_empty())
                {
                    if origin == "*" && production {
                        tracing::warn!(
                            "Ignoring wildcard CORS_ORIGINS=* in production; configure explicit origins instead"
                        );
                        continue;
                    }
                    origins.push(cors_origin(origin, production)?);
                }
                Ok(origins)
            })
            .transpose()?
            .unwrap_or_default();

        let postgres = PgConnectOptions::new()
            .host(&non_empty(&lookup, "DB_HOST", "localhost"))
            .port(parse(&lookup, "DB_PORT", 5432)?)
            .username(&non_empty(&lookup, "DB_USER", "postgres"))
            .password(&lookup("DB_PASSWORD").unwrap_or_else(|| "postgres".into()))
            .database(&non_empty(&lookup, "DB_NAME", "url_shortener"));

        let sample_rate = parse(&lookup, "SENTRY_TRACES_SAMPLE_RATE", 0.1_f32)?;
        if !(0.0..=1.0).contains(&sample_rate) {
            return Err(ConfigError("SENTRY_TRACES_SAMPLE_RATE"));
        }

        Ok(Self {
            bind: SocketAddr::from(([0, 0, 0, 0], server_port)),
            production,
            postgres,
            db_max_connections,
            cache_max_capacity_bytes: parse(&lookup, "CACHE_MAX_CAPACITY_BYTES", 32 * 1024 * 1024)?,
            cors_origins,
            rate_limit_per_second,
            rate_limit_burst_size,
            run_migrations: parse_bool(&lookup, "RUN_MIGRATIONS", true)?,
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

fn non_empty(lookup: &impl Fn(&str) -> Option<String>, name: &str, default: &str) -> String {
    lookup(name)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.into())
}

fn parse<T: std::str::FromStr>(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: T,
) -> Result<T, ConfigError> {
    match lookup(name) {
        Some(value) if !value.trim().is_empty() => {
            value.trim().parse().map_err(|_| ConfigError(name))
        }
        _ => Ok(default),
    }
}

fn parse_bool(
    lookup: &impl Fn(&str) -> Option<String>,
    name: &'static str,
    default: bool,
) -> Result<bool, ConfigError> {
    match lookup(name)
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None => Ok(default),
        Some("true" | "1" | "yes") => Ok(true),
        Some("false" | "0" | "no") => Ok(false),
        Some(_) => Err(ConfigError(name)),
    }
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
    fn validates_numeric_and_env_bounds() {
        for (name, value) in [
            ("RUST_ENV", "staging"),
            ("SERVER_PORT", "abc"),
            ("SERVER_PORT", "0"),
            ("DB_MAX_CONNECTIONS", "0"),
            ("RATE_LIMIT_PER_SECOND", "0"),
            ("RATE_LIMIT_BURST_SIZE", "0"),
            ("SENTRY_TRACES_SAMPLE_RATE", "1.5"),
            ("RUN_MIGRATIONS", "maybe"),
        ] {
            assert!(config(&[(name, value)]).is_err(), "{name}={value}");
        }
    }

    #[test]
    fn tolerates_empty_defaults_case_insensitive_env_and_prod_wildcard_cors() {
        let cfg = config(&[
            ("RUST_ENV", "Production"),
            ("RUN_MIGRATIONS", "1"),
            ("SERVER_PORT", ""),
            ("CORS_ORIGINS", "*, https://u.lou2.kr"),
        ])
        .unwrap();
        assert!(cfg.production);
        assert!(cfg.run_migrations);
        assert_eq!(cfg.bind.port(), 3000);
        assert_eq!(cfg.cors_origins.len(), 1);
        assert_eq!(cfg.cors_origins[0], "https://u.lou2.kr");
    }

    #[test]
    fn ignores_removed_legacy_env_vars() {
        let cfg = config(&[
            ("REDIS_HOST", "redis"),
            ("JWT_SECRET", "legacy-unused-secret"),
            ("CACHE_TTL_SECS", "3600"),
        ])
        .unwrap();
        assert_eq!(cfg.bind.port(), 3000);
    }
}
