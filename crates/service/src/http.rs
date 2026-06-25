use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::Value;
use tokio::net::TcpListener;

use crate::config::ServiceConfig;
use crate::error::AppError;
use crate::query_service::QueryService;
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
        .route("/health", get(routes::health::health))
        .route("/ready", get(routes::health::ready))
        .route("/query", post(routes::query::query))
        .route("/batch", post(routes::query::batch))
        .route("/prewarm", post(routes::query::prewarm))
        .route("/concrete-lines", post(routes::metadata::concrete_lines))
        .route(
            "/drill-scenario-lines",
            post(routes::metadata::drill_scenario_lines),
        )
        .with_state(state)
}

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
}

impl From<AppError> for HttpError {
    fn from(error: AppError) -> Self {
        Self {
            error,
            details: None,
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.error.code() {
            "UNKNOWN_HAND" | "INVALID_ARGUMENT" => StatusCode::BAD_REQUEST,
            "BIN_FILE_NOT_FOUND" | "PACK_NOT_FOUND" => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorBody {
            code: self.error.code(),
            message: self.error.message(),
            details: self.details.as_ref(),
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::builder::{build_store, BuildOptions, DimensionSpec};
    use crate::sqlite::Connection;

    #[tokio::test]
    async fn serves_query_and_metadata_workflows() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = build_test_store(directory.path());
        let service = Arc::new(QueryService::open(&data_dir, 2, true).unwrap());
        let app = router(service);

        let (status, health) = call_json(&app, Method::GET, "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(health["status"], "ok");

        let (status, ready) = call_json(&app, Method::GET, "/ready", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(ready["dimensions_known"][0], "default_6max_100BB");

        let query = json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "concrete_line_id": 1,
            "hole_cards": "AsAh"
        });
        let (status, result) = call_json(&app, Method::POST, "/query", Some(query)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["hand_code"], "AA");
        assert_eq!(result["exists"], true);
        assert_eq!(result["actions"].as_array().unwrap().len(), 2);

        let invalid_query = json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "concrete_line_id": 1,
            "hole_cards": "AsXx"
        });
        let (status, error) = call_json(&app, Method::POST, "/query", Some(invalid_query)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["code"], "UNKNOWN_HAND");

        let batch = json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "requests": [
                { "concrete_line_id": 1, "hole_cards": "AA" },
                { "concrete_line_id": 1, "hole_cards": "AsXx" }
            ]
        });
        let (status, result) = call_json(&app, Method::POST, "/batch", Some(batch)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["results"][0]["strategy"]["exists"], true);
        assert!(result["results"][0]["strategy"].get("hand_code").is_none());
        assert_eq!(result["results"][1]["error"]["code"], "UNKNOWN_HAND");

        let prewarm = json!({
            "dimensions": [
                { "strategy": "default", "player_count": 6, "depth_bb": 100 }
            ]
        });
        let (status, result) = call_json(&app, Method::POST, "/prewarm", Some(prewarm)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["prewarmed"], 1);
        assert_eq!(result["total_open"], 1);

        let concrete_lines = json!({
            "strategy": "default",
            "player_count": 6,
            "depth_bb": 100,
            "abstract_line": "F-F-F"
        });
        let (status, result) =
            call_json(&app, Method::POST, "/concrete-lines", Some(concrete_lines)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["lines"][0]["concrete_line_id"], 1);

        let drill_lines = json!({
            "strategy": "default",
            "drill_name": "UTG",
            "player_count": 6,
            "drill_depth": 0
        });
        let (status, result) = call_json(
            &app,
            Method::POST,
            "/drill-scenario-lines",
            Some(drill_lines),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["abstract_lines"], json!(["F-F-F"]));
    }

    async fn call_json(
        app: &Router,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(path);
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&value).unwrap())
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    fn build_test_store(root: &Path) -> std::path::PathBuf {
        let source_path = root.join("source.db");
        let output_path = root.join("output");
        let source = Connection::open(&source_path, false).unwrap();
        source
            .exec(
                "CREATE TABLE range_data_default_6max_100BB (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   concrete_line_id INTEGER NOT NULL,
                   hole_cards TEXT NOT NULL,
                   action_name TEXT NOT NULL,
                   action_size REAL NOT NULL,
                   amount_bb REAL NOT NULL,
                   frequency REAL NOT NULL,
                   hand_ev REAL NULL
                 );
                 CREATE TABLE concrete_lines_default_6max_100BB (
                   id INTEGER PRIMARY KEY,
                   abstract_line TEXT NOT NULL,
                   concrete_line TEXT NOT NULL
                 );
                 CREATE TABLE drill_scenario_lines_default (
                   id INTEGER PRIMARY KEY,
                   drill_name TEXT NOT NULL,
                   abstract_line TEXT NOT NULL,
                   player_count INTEGER NOT NULL,
                   depth INTEGER NOT NULL
                 );
                 INSERT INTO concrete_lines_default_6max_100BB
                   VALUES (1, 'F-F-F', 'F-F-F');
                 INSERT INTO drill_scenario_lines_default
                   VALUES (1, 'UTG', 'F-F-F', 6, 0);
                 INSERT INTO range_data_default_6max_100BB(
                   concrete_line_id, hole_cards, action_name, action_size,
                   amount_bb, frequency, hand_ev
                 ) VALUES
                   (1, 'AA', 'fold', 0, 0, 0.25, NULL),
                   (1, 'AA', 'raise', 2.5, 2.5, 0.75, 1.0);",
            )
            .unwrap();
        drop(source);

        build_store(&BuildOptions {
            source_db: source_path,
            out_dir: output_path.clone(),
            dimensions: vec![DimensionSpec {
                strategy: "default".to_owned(),
                player_count: 6,
                depth_bb: 100,
            }],
            max_concrete_lines_per_dimension: None,
            overwrite: false,
        })
        .unwrap();
        output_path
    }
}
