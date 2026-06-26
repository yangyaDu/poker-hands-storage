use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::http::AppState;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    /// Health status. `ok` means the HTTP process is running.
    status: &'static str,
    /// Process uptime in seconds.
    uptime_secs: f64,
}

#[derive(Serialize, ToSchema)]
pub struct ReadyResponse {
    /// Readiness status. `ready` means the data store was opened.
    status: &'static str,
    /// Number of action schemas loaded from metadata.
    schema_count: usize,
    /// Number of currently open dimension handles.
    handles_open: usize,
    /// Queryable dimensions known from the manifest.
    dimensions_known: Vec<String>,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service liveness status.", body = HealthResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        uptime_secs: state.started_at.elapsed().as_secs_f64(),
    })
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "health",
    responses(
        (status = 200, description = "Service readiness and loaded data summary.", body = ReadyResponse)
    )
)]
pub async fn ready(State(state): State<AppState>) -> Json<ReadyResponse> {
    Json(ReadyResponse {
        status: "ready",
        schema_count: state.service.schema_count(),
        handles_open: state.service.open_handle_count(),
        dimensions_known: state.service.known_dimensions(),
    })
}
