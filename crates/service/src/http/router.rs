use std::sync::Arc;
use std::time::Instant;

use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use utoipa::OpenApi;

use crate::http::app_state::AppState;
use crate::http::openapi::ApiDoc;
use crate::query::QueryService;
use crate::routes;

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
