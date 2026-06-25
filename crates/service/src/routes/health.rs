use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::http::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    uptime_secs: f64,
}

#[derive(Serialize)]
pub struct ReadyResponse {
    status: &'static str,
    schema_count: usize,
    handles_open: usize,
    dimensions_known: Vec<String>,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        uptime_secs: state.started_at.elapsed().as_secs_f64(),
    })
}

pub async fn ready(State(state): State<AppState>) -> Json<ReadyResponse> {
    Json(ReadyResponse {
        status: "ready",
        schema_count: state.service.schema_count(),
        handles_open: state.service.open_handle_count(),
        dimensions_known: state.service.known_dimensions(),
    })
}
