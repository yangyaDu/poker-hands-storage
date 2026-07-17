use std::sync::Arc;

use tokio::net::TcpListener;

use crate::config::ServiceConfig;
use crate::errors::AppError;
use crate::http::router::router;
use crate::query::QueryService;

pub async fn serve(config: ServiceConfig) -> Result<(), AppError> {
    let service = Arc::new(QueryService::open_with_options(
        &config.data_dir,
        config.max_open_handles,
        config.verify_checksums,
        config.metadata_cache_byte_budget,
        config.strategy_cache_byte_budget,
    )?);
    for dimension in &config.prewarm {
        service.prewarm(dimension)?;
    }

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(
        bind = %config.bind,
        data_dir = %config.data_dir.display(),
        known_dimensions = service.known_dimensions().len(),
        prewarmed_handles = service.open_handle_count(),
        "poker-hands-storage service ready"
    );
    axum::serve(listener, router(service))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(AppError::from)
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}
