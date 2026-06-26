use std::sync::Arc;
use std::time::Instant;

pub mod openapi;
pub mod request_validation;

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;
use utoipa::OpenApi;
use utoipa::ToSchema;

use crate::config::ServiceConfig;
use crate::errors::AppError;
use crate::http::openapi::ApiDoc;
use crate::http::request_validation::ValidationErrorDetails;
use crate::query::QueryService;
use crate::routes;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<QueryService>,
    pub started_at: Instant,
}

pub fn router(service: Arc<QueryService>) -> Router {
    let state = AppState {
        service,
        started_at: Instant::now(),
    };
    Router::new()
        .route("/swagger", get(swagger_page))
        .route("/swagger/", get(swagger_page))
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/health", get(routes::health_routes::health))
        .route("/ready", get(routes::health_routes::ready))
        .route(
            "/range/hand-strategy",
            post(routes::hand_query_routes::query),
        )
        .route(
            "/range/hand-strategy/batch",
            post(routes::hand_query_routes::batch),
        )
        .route(
            "/range/hands-by-actions",
            post(routes::hand_query_routes::hands_by_actions),
        )
        .route("/range/prewarm", post(routes::hand_query_routes::prewarm))
        .route(
            "/range/concrete-lines",
            post(routes::metadata_routes::concrete_lines),
        )
        .route(
            "/range/drill-scenarios",
            post(routes::metadata_routes::drill_scenario_lines),
        )
        .with_state(state)
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

async fn swagger_page() -> Html<&'static str> {
    Html(SWAGGER_HTML)
}

const SWAGGER_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Poker Hands Storage API</title>
    <style>
      body {
        margin: 0;
      }
    </style>
  </head>
  <body>
    <div id="app"></div>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
    <script>
      Scalar.createApiReference('#app', {
        url: '/api-docs/openapi.json',
        layout: 'modern',
        theme: 'default',
        hideDownloadButton: false,
        metaData: {
          title: 'Poker Hands Storage API'
        }
      })
    </script>
  </body>
</html>"#;

pub async fn serve(config: ServiceConfig) -> Result<(), AppError> {
    let service = Arc::new(QueryService::open_with_meta(
        &config.data_dir,
        &config.meta_db,
        config.max_open_handles,
        config.verify_checksums,
    )?);
    for dimension in &config.prewarm {
        service.prewarm(dimension)?;
    }

    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(
        bind = %config.bind,
        data_dir = %config.data_dir.display(),
        meta_db = %config.meta_db.display(),
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

#[derive(Debug)]
pub struct HttpError {
    error: AppError,
    details: Option<Value>,
}

impl HttpError {
    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self {
            error: AppError::invalid_argument(message),
            details: None,
        }
    }

    pub fn validation(details: ValidationErrorDetails) -> Self {
        Self {
            error: AppError::invalid_argument("request validation failed"),
            details: serde_json::to_value(details).ok(),
        }
    }
}

impl From<AppError> for HttpError {
    fn from(error: AppError) -> Self {
        Self {
            error,
            details: None,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Stable application error code.
    code: String,
    /// Human-readable error message.
    message: String,
    /// Optional structured error details, usually field validation failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<ValidationErrorDetails>)]
    details: Option<Value>,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.error.code() {
            "UNKNOWN_HAND" | "INVALID_ARGUMENT" => StatusCode::BAD_REQUEST,
            "BIN_FILE_NOT_FOUND" | "PACK_NOT_FOUND" => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorResponse {
            code: self.error.code().to_owned(),
            message: self.error.message().to_owned(),
            details: self.details,
        };
        (status, Json(body)).into_response()
    }
}
