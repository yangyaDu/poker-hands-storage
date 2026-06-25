use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::http::{AppState, HttpError};
use crate::meta_db::ConcreteLineRow;
use crate::naming::DimensionRef;

use super::run_blocking;

#[derive(Deserialize)]
pub struct ConcreteLinesRequest {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    abstract_line: String,
}

#[derive(Serialize)]
pub struct ConcreteLinesResponse {
    lines: Vec<ConcreteLineRow>,
}

#[derive(Deserialize)]
pub struct DrillScenarioLinesRequest {
    strategy: String,
    drill_name: String,
    player_count: u32,
    drill_depth: u32,
}

#[derive(Serialize)]
pub struct DrillScenarioLinesResponse {
    abstract_lines: Vec<String>,
}

pub async fn concrete_lines(
    State(state): State<AppState>,
    payload: Result<Json<ConcreteLinesRequest>, JsonRejection>,
) -> Result<Json<ConcreteLinesResponse>, HttpError> {
    let request = payload
        .map_err(|error| HttpError::invalid_json(error.body_text()))?
        .0;
    let service = state.service;
    run_blocking(move || {
        let dimension = DimensionRef::new(request.strategy, request.player_count, request.depth_bb);
        service
            .get_concrete_lines(&dimension, &request.abstract_line)
            .map(|lines| ConcreteLinesResponse { lines })
    })
    .await
    .map(Json)
}

pub async fn drill_scenario_lines(
    State(state): State<AppState>,
    payload: Result<Json<DrillScenarioLinesRequest>, JsonRejection>,
) -> Result<Json<DrillScenarioLinesResponse>, HttpError> {
    let request = payload
        .map_err(|error| HttpError::invalid_json(error.body_text()))?
        .0;
    let service = state.service;
    run_blocking(move || {
        service
            .get_drill_scenario_lines(
                &request.strategy,
                &request.drill_name,
                request.player_count,
                request.drill_depth,
            )
            .map(|abstract_lines| DrillScenarioLinesResponse { abstract_lines })
    })
    .await
    .map(Json)
}
