use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::http::{AppState, HttpError};
use crate::naming::DimensionRef;
use crate::query_service::{BatchItemResult, QueryResult};

use super::{run_blocking, run_infallible_blocking};

#[derive(Deserialize)]
pub struct QueryRequest {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line_id: u32,
    hole_cards: String,
}

#[derive(Deserialize)]
pub struct BatchRequest {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    requests: Vec<BatchQueryItem>,
}

#[derive(Deserialize)]
pub struct BatchQueryItem {
    concrete_line_id: u32,
    hole_cards: String,
}

#[derive(Serialize)]
pub struct BatchResponse {
    results: Vec<BatchItemResult>,
}

#[derive(Deserialize)]
pub struct PrewarmRequest {
    dimensions: Vec<DimensionRequest>,
}

#[derive(Deserialize)]
pub struct DimensionRequest {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
}

#[derive(Serialize)]
pub struct PrewarmResponse {
    prewarmed: usize,
    total_open: usize,
}

pub async fn query(
    State(state): State<AppState>,
    payload: Result<Json<QueryRequest>, JsonRejection>,
) -> Result<Json<QueryResult>, HttpError> {
    let request = payload
        .map_err(|error| HttpError::invalid_json(error.body_text()))?
        .0;
    let service = state.service;
    run_blocking(move || {
        service.query(
            &DimensionRef::new(request.strategy, request.player_count, request.depth_bb),
            request.concrete_line_id,
            &request.hole_cards,
        )
    })
    .await
    .map(Json)
}

pub async fn batch(
    State(state): State<AppState>,
    payload: Result<Json<BatchRequest>, JsonRejection>,
) -> Result<Json<BatchResponse>, HttpError> {
    let request = payload
        .map_err(|error| HttpError::invalid_json(error.body_text()))?
        .0;
    let service = state.service;
    let dimension = DimensionRef::new(request.strategy, request.player_count, request.depth_bb);
    let requests: Vec<_> = request
        .requests
        .into_iter()
        .map(|item| (item.concrete_line_id, item.hole_cards))
        .collect();
    let results =
        run_infallible_blocking(move || service.query_batch(&dimension, &requests)).await?;
    Ok(Json(BatchResponse { results }))
}

pub async fn prewarm(
    State(state): State<AppState>,
    payload: Result<Json<PrewarmRequest>, JsonRejection>,
) -> Result<Json<PrewarmResponse>, HttpError> {
    let request = payload
        .map_err(|error| HttpError::invalid_json(error.body_text()))?
        .0;
    let service = state.service;
    run_blocking(move || {
        for dimension in &request.dimensions {
            service.prewarm(&DimensionRef::new(
                dimension.strategy.clone(),
                dimension.player_count,
                dimension.depth_bb,
            ))?;
        }
        Ok(PrewarmResponse {
            prewarmed: request.dimensions.len(),
            total_open: service.open_handle_count(),
        })
    })
    .await
    .map(Json)
}
