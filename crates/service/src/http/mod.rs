use std::sync::Arc;
use std::time::Instant;

pub mod healthcheck;
pub mod openapi;
pub mod request_validation;

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
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

/// Unified success response envelope: `{ code, data, message }`.
/// `code == 0` means success; non-zero means a business error.
#[derive(Serialize, ToSchema)]
pub struct ApiResponse<T> {
    /// Business status code. 0 indicates success.
    code: i32,
    /// Typed response payload (null on error).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<T>)]
    data: Option<T>,
    /// Human-readable message. Success responses use null; error responses carry a message.
    #[schema(nullable)]
    message: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            code: 0,
            data: Some(data),
            message: None,
        }
    }
}

/// Public API status codes. Detailed failure reasons belong in `message`.
pub mod error_code {
    pub const SUCCESS: i32 = 0;
    pub const BAD_REQUEST: i32 = 1000;
    pub const NOT_FOUND: i32 = 404;
    pub const INTERNAL_SERVER_ERROR: i32 = 500;
    pub const SERVICE_UNAVAILABLE: i32 = 503;
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
            "/range/hand-strategy-batch",
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
}

impl HttpError {
    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self {
            error: AppError::invalid_argument(message),
        }
    }

    pub fn validation(details: ValidationErrorDetails) -> Self {
        Self {
            error: AppError::invalid_argument(format!(
                "request validation failed: {}",
                details.message()
            )),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            error: AppError::invalid_argument(message),
        }
    }
}

impl From<AppError> for HttpError {
    fn from(error: AppError) -> Self {
        Self { error }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponseBody {
    /// Business status code. Non-zero values indicate errors.
    code: i32,
    /// Error responses do not carry endpoint data.
    #[schema(nullable)]
    data: Option<()>,
    /// Human-readable message.
    message: String,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let code = self.error.public_code();
        let status = match code {
            error_code::BAD_REQUEST => StatusCode::BAD_REQUEST,
            error_code::NOT_FOUND => StatusCode::NOT_FOUND,
            error_code::SERVICE_UNAVAILABLE => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorResponseBody {
            code,
            data: None,
            message: self.error.message().to_owned(),
        };
        (status, Json(body)).into_response()
    }
}
