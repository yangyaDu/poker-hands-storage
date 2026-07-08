use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

use crate::http::blocking_task::run_blocking;
use crate::http::request_validation::{
    validate_allowed_str, validate_allowed_u32, validate_required_string, ValidateRequest,
    ValidatedJson, ValidationErrorDetails, ALLOWED_DEPTH_BB, ALLOWED_PLAYER_COUNTS,
    ALLOWED_STRATEGIES,
};
use crate::http::{ApiResponse, AppState, HttpError};
use range_store_core::dimension::DimensionRef;
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};

#[derive(Deserialize, ToSchema)]
pub struct ConcreteLinesRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    #[serde(default = "default_strategy")]
    strategy: String,
    /// Number of players for the target game tree.
    #[schema(example = 6, minimum = 1)]
    #[serde(default = "default_player_count")]
    player_count: u32,
    /// Stack depth in big blinds.
    #[schema(example = 100, minimum = 1)]
    #[serde(default = "default_depth_bb")]
    depth_bb: u32,
    /// Abstract action line used to find concrete lines.
    #[schema(example = "F-F-F", nullable = false)]
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    abstract_line: Option<String>,
    /// Concrete action line used for exact concrete-line lookup.
    #[schema(example = "F-F-F-R2-F-R7-R15", nullable = false)]
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    concrete_line: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ConcreteLinesPayload {
    /// Concrete lines matching the requested abstract line.
    pub lines: Vec<ConcreteLineRow>,
}

#[derive(Deserialize, ToSchema)]
pub struct DrillScenarioLinesRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    #[serde(default = "default_strategy")]
    strategy: String,
    /// Drill scenario name.
    #[schema(example = "rfi", default = "rfi")]
    #[serde(default = "default_drill_name")]
    drill_name: String,
    /// Number of players for the target drill.
    #[schema(example = 6, minimum = 1)]
    #[serde(default = "default_player_count")]
    player_count: u32,
    /// Drill depth bucket.
    #[schema(example = 100)]
    #[serde(default = "default_depth_bb")]
    drill_depth: u32,
}

#[derive(Serialize, ToSchema)]
pub struct DrillScenarioLinesPayload {
    /// Abstract lines available for the requested drill scenario.
    pub abstract_lines: Vec<String>,
}

impl ValidateRequest for ConcreteLinesRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_allowed_str(
            &mut errors,
            "strategy",
            &self.strategy,
            ALLOWED_STRATEGIES,
            "must be one of \"default\"",
        );
        validate_allowed_u32(
            &mut errors,
            "player_count",
            self.player_count,
            ALLOWED_PLAYER_COUNTS,
            "must be one of 6, 8, 9",
        );
        validate_allowed_u32(
            &mut errors,
            "depth_bb",
            self.depth_bb,
            ALLOWED_DEPTH_BB,
            "must be one of 100, 200, 300",
        );
        if self.abstract_line.is_none() && self.concrete_line.is_none() {
            errors.push(
                "abstract_line/concrete_line",
                "one of abstract_line or concrete_line must be provided",
            );
        }
        if let Some(abstract_line) = &self.abstract_line {
            validate_required_string(&mut errors, "abstract_line", abstract_line);
        }
        if let Some(concrete_line) = &self.concrete_line {
            validate_required_string(&mut errors, "concrete_line", concrete_line);
        }
        errors.finish()
    }
}

impl ValidateRequest for DrillScenarioLinesRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_allowed_str(
            &mut errors,
            "strategy",
            &self.strategy,
            ALLOWED_STRATEGIES,
            "must be one of \"default\"",
        );
        validate_required_string(&mut errors, "drill_name", &self.drill_name);
        validate_drill_name(&mut errors, &self.drill_name);
        validate_allowed_u32(
            &mut errors,
            "player_count",
            self.player_count,
            ALLOWED_PLAYER_COUNTS,
            "must be one of 6, 8, 9",
        );
        validate_allowed_u32(
            &mut errors,
            "drill_depth",
            self.drill_depth,
            ALLOWED_DEPTH_BB,
            "must be one of 100, 200, 300",
        );
        errors.finish()
    }
}

fn validate_drill_name(errors: &mut ValidationErrorDetails, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return;
    };
    if !first.is_ascii_alphabetic() {
        errors.push("drill_name", "must start with a letter");
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        errors.push("drill_name", "must not be numeric-only");
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        errors.push(
            "drill_name",
            "may only contain letters, digits, underscore, or hyphen",
        );
    }
}

fn default_strategy() -> String {
    "default".to_owned()
}

fn default_player_count() -> u32 {
    6
}

fn default_depth_bb() -> u32 {
    100
}

fn default_drill_name() -> String {
    "rfi".to_owned()
}

fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(Some)
}

#[utoipa::path(
    post,
    path = "/range/concrete-lines",
    tag = "range",
    request_body(content = ConcreteLinesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Concrete lines for an abstract action line.", body = crate::http::openapi::ConcreteLinesResponseEnvelope),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Concrete lines not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn concrete_lines(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<ConcreteLinesRequest>,
) -> Result<Json<ApiResponse<ConcreteLinesPayload>>, HttpError> {
    concrete_lines_impl(state, request).await
}

async fn concrete_lines_impl(
    state: AppState,
    request: ConcreteLinesRequest,
) -> Result<Json<ApiResponse<ConcreteLinesPayload>>, HttpError> {
    let service = state.service;
    let response = run_blocking(move || {
        let dimension = DimensionRef::new(request.strategy, request.player_count, request.depth_bb);
        let filter = match (
            request.abstract_line.as_deref(),
            request.concrete_line.as_deref(),
        ) {
            (Some(abstract_line), Some(concrete_line)) => ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            },
            (Some(abstract_line), None) => ConcreteLineFilter::Abstract(abstract_line),
            (None, Some(concrete_line)) => ConcreteLineFilter::Concrete(concrete_line),
            (None, None) => unreachable!("ConcreteLinesRequest validation requires a line filter"),
        };
        service
            .get_concrete_lines(&dimension, filter)
            .map(|lines| ConcreteLinesPayload { lines })
    })
    .await?;
    Ok(Json(ApiResponse::ok(response)))
}

#[utoipa::path(
    post,
    path = "/range/drill-scenarios",
    tag = "range",
    request_body(content = DrillScenarioLinesRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Abstract lines for a drill scenario.", body = crate::http::openapi::DrillScenarioLinesResponseEnvelope),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Drill scenario abstract lines not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn drill_scenario_lines(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<DrillScenarioLinesRequest>,
) -> Result<Json<ApiResponse<DrillScenarioLinesPayload>>, HttpError> {
    drill_scenario_lines_impl(state, request).await
}

async fn drill_scenario_lines_impl(
    state: AppState,
    request: DrillScenarioLinesRequest,
) -> Result<Json<ApiResponse<DrillScenarioLinesPayload>>, HttpError> {
    let service = state.service;
    let response = run_blocking(move || {
        service
            .get_drill_scenario_lines(
                &request.strategy,
                &request.drill_name,
                request.player_count,
                request.drill_depth,
            )
            .map(|abstract_lines| DrillScenarioLinesPayload { abstract_lines })
    })
    .await?;
    Ok(Json(ApiResponse::ok(response)))
}
