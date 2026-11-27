use std::{net::SocketAddr, time::Duration};

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;
use url_shortener::{
    config::Config,
    handlers::{create_routes, AppState},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env()?;
    let _sentry = if config.sentry_dsn.is_empty() {
        None
    } else {
        Some(sentry::init(sentry::ClientOptions {
            dsn: Some(config.sentry_dsn.parse()?),
            release: sentry::release_name!(),
            traces_sample_rate: config.sentry_traces_sample_rate,
            shutdown_timeout: Duration::from_secs(2),
            ..Default::default()
        }))
    };

    let db = config.connect_db().await?;
    if config.run_migrations {
        let invalid_index: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_index
             WHERE indexrelid = to_regclass('idx_urls_hashed_value_active_unique')
             AND NOT indisvalid)",
        )
        .fetch_one(&db)
        .await?;
        if invalid_index {
            return Err(
                "Interrupted active-hash index migration: drop the invalid index before retrying"
                    .into(),
            );
        }
        sqlx::migrate!().run(&db).await?;
    }
    let state = AppState::new(db, &config).await?;
    let listener = TcpListener::bind(config.bind).await?;
    let routes = create_routes(state.clone(), &config)?;
    tracing::info!(address = %listener.local_addr()?, "URL shortener listening");
    let result = axum::serve(
        listener,
        routes.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await;
    state.shutdown().await;
    result?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            },
            Err(error) => {
                tracing::error!(%error, "Installing SIGTERM handler failed");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
