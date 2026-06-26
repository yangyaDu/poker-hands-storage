use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::http::{AppState, HttpError};
use crate::meta_db::ConcreteLineRow;
use crate::naming::DimensionRef;
use crate::validation::{
    validate_positive_u32, validate_required_string, ValidateRequest, ValidatedJson,
    ValidationErrorDetails,
};

use super::run_blocking;

#[derive(Deserialize, ToSchema)]
pub struct ConcreteLinesRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    strategy: String,
    /// Number of players for the target game tree.
    #[schema(example = 6, minimum = 1)]
    player_count: u32,
    /// Stack depth in big blinds.
    #[schema(example = 100, minimum = 1)]
    depth_bb: u32,
    /// Abstract action line used to find concrete lines.
    #[schema(example = "F-F-F")]
    abstract_line: String,
}

#[derive(Serialize, ToSchema)]
pub struct ConcreteLinesResponse {
    /// Concrete lines matching the requested abstract line.
    lines: Vec<ConcreteLineRow>,
}

#[derive(Deserialize, ToSchema)]
pub struct DrillScenarioLinesRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    strategy: String,
    /// Drill scenario name.
    #[schema(example = "UTG")]
    drill_name: String,
    /// Number of players for the target drill.
    #[schema(example = 6, minimum = 1)]
    player_count: u32,
    /// Drill depth bucket.
    #[schema(example = 0)]
    drill_depth: u32,
}

#[derive(Serialize, ToSchema)]
pub struct DrillScenarioLinesResponse {
    /// Abstract lines available for the requested drill scenario.
    abstract_lines: Vec<String>,
}

impl ValidateRequest for ConcreteLinesRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_required_string(&mut errors, "strategy", &self.strategy);
        validate_positive_u32(&mut errors, "player_count", self.player_count);
        validate_positive_u32(&mut errors, "depth_bb", self.depth_bb);
        validate_required_string(&mut errors, "abstract_line", &self.abstract_line);
        errors.finish()
    }
}

impl ValidateRequest for DrillScenarioLinesRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_required_string(&mut errors, "strategy", &self.strategy);
        validate_required_string(&mut errors, "drill_name", &self.drill_name);
        validate_positive_u32(&mut errors, "player_count", self.player_count);
        errors.finish()
    }
}

#[utoipa::path(
    post,
    path = "/concrete-lines",
    tag = "metadata",
    request_body(content = ConcreteLinesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Concrete lines for an abstract action line.", body = ConcreteLinesResponse),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::ErrorResponse)
    )
)]
pub async fn concrete_lines(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<ConcreteLinesRequest>,
) -> Result<Json<ConcreteLinesResponse>, HttpError> {
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

#[utoipa::path(
    post,
    path = "/drill-scenario-lines",
    tag = "metadata",
    request_body(content = DrillScenarioLinesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Abstract lines for a drill scenario.", body = DrillScenarioLinesResponse),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::ErrorResponse)
    )
)]
pub async fn drill_scenario_lines(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<DrillScenarioLinesRequest>,
) -> Result<Json<DrillScenarioLinesResponse>, HttpError> {
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
