use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::action_schema::ActionName;
use crate::domain::dimension::DimensionRef;
use crate::http::request_validation::{
    validate_allowed_str, validate_allowed_u32, validate_positive_u32, validate_required_string,
    ValidateRequest, ValidatedJson, ValidationErrorDetails, ALLOWED_DEPTH_BB,
    ALLOWED_PLAYER_COUNTS, ALLOWED_STRATEGIES, MAX_BATCH_REQUESTS, MAX_PREWARM_DIMENSIONS,
};
use crate::http::{ApiResponse, AppState, HttpError};
use crate::query::{ActionFilter, BatchItemResult, HandsByActionsResult, QueryResult};

use super::run_blocking;

#[derive(Deserialize, ToSchema)]
pub struct QueryRequest {
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
    /// Concrete line id from the selected dimension.
    #[schema(example = 1, minimum = 1)]
    #[serde(default)]
    concrete_line_id: u32,
    /// Hole cards as a 169-hand code or two-card code, for example `AA`, `AKs`, `AKo`, or `AsKh`.
    #[schema(example = "AA")]
    #[serde(default)]
    hole_cards: String,
}

#[derive(Deserialize, ToSchema)]
pub struct BatchRequest {
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
    /// Query items to resolve in one dimension. Maximum 500 items per request.
    #[schema(max_items = 500)]
    #[serde(default)]
    requests: Vec<BatchQueryItem>,
}

#[derive(Deserialize, ToSchema)]
pub struct BatchQueryItem {
    /// Concrete line id from the selected dimension.
    #[schema(example = 1, minimum = 1)]
    #[serde(default)]
    concrete_line_id: u32,
    /// Hole cards as a 169-hand code or two-card code.
    #[schema(example = "AA")]
    #[serde(default)]
    hole_cards: String,
}

#[derive(Serialize, ToSchema)]
pub struct BatchPayload {
    /// Per-item query result. Invalid hand inputs are reported on their individual item.
    results: Vec<BatchItemResult>,
}

#[derive(Deserialize, ToSchema)]
pub struct HandsByActionsRequest {
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
    /// Concrete line id from the selected dimension.
    #[schema(example = 1, minimum = 1)]
    #[serde(default)]
    concrete_line_id: u32,
    /// Optional list of action-name filters. Omitted or empty means all hands in the concrete line.
    /// Supported names: fold, call, check, bet, raise, allin. Amount suffixes such as raise2.5
    /// are also accepted for exact action-size filtering.
    #[schema(nullable, example = json!(["fold", "raise"]))]
    actions: Option<Vec<String>>,
    /// Optional frequency threshold. Only hands with frequency >= this value are included.
    #[schema(nullable, example = 0.005, default = 0.0)]
    frequency: Option<f64>,
}

#[derive(Deserialize, ToSchema)]
pub struct PrewarmRequest {
    /// Dimensions to open and validate in the handle pool. Maximum 64 dimensions per request.
    #[schema(max_items = 64)]
    #[serde(default)]
    dimensions: Vec<DimensionRequest>,
}

#[derive(Deserialize, ToSchema)]
pub struct DimensionRequest {
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
}

#[derive(Serialize, ToSchema)]
pub struct PrewarmPayload {
    /// Number of dimensions requested for prewarm.
    prewarmed: usize,
    /// Total number of dimension handles open after prewarm.
    total_open: usize,
}

impl ValidateRequest for QueryRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_dimension_fields(
            &mut errors,
            "strategy",
            &self.strategy,
            "player_count",
            self.player_count,
            "depth_bb",
            self.depth_bb,
        );
        validate_positive_u32(&mut errors, "concrete_line_id", self.concrete_line_id);
        validate_required_string(&mut errors, "hole_cards", &self.hole_cards);
        errors.finish()
    }
}

impl ValidateRequest for BatchRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_dimension_fields(
            &mut errors,
            "strategy",
            &self.strategy,
            "player_count",
            self.player_count,
            "depth_bb",
            self.depth_bb,
        );
        if self.requests.is_empty() {
            errors.push("requests", "must contain at least one item");
        }
        if self.requests.len() > MAX_BATCH_REQUESTS {
            errors.push(
                "requests",
                format!("must contain at most {MAX_BATCH_REQUESTS} items"),
            );
        }
        for (index, item) in self.requests.iter().enumerate() {
            validate_batch_item(&mut errors, index, item);
        }
        errors.finish()
    }
}

impl ValidateRequest for HandsByActionsRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        validate_dimension_fields(
            &mut errors,
            "strategy",
            &self.strategy,
            "player_count",
            self.player_count,
            "depth_bb",
            self.depth_bb,
        );
        validate_positive_u32(&mut errors, "concrete_line_id", self.concrete_line_id);
        if let Some(ref actions) = self.actions {
            for (i, action) in actions.iter().enumerate() {
                if let Err(e) = parse_action_filter(action) {
                    errors.push(format!("actions[{i}]"), e.to_string());
                }
            }
        }
        if let Some(frequency) = self.frequency {
            if !(0.0..=1.0).contains(&frequency) {
                errors.push("frequency", "must be between 0 and 1");
            }
        }
        errors.finish()
    }
}

impl ValidateRequest for PrewarmRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails> {
        let mut errors = ValidationErrorDetails::new();
        if self.dimensions.is_empty() {
            errors.push("dimensions", "must contain at least one item");
        }
        if self.dimensions.len() > MAX_PREWARM_DIMENSIONS {
            errors.push(
                "dimensions",
                format!("must contain at most {MAX_PREWARM_DIMENSIONS} items"),
            );
        }
        for (index, dimension) in self.dimensions.iter().enumerate() {
            validate_dimension_fields(
                &mut errors,
                format!("dimensions[{index}].strategy"),
                &dimension.strategy,
                format!("dimensions[{index}].player_count"),
                dimension.player_count,
                format!("dimensions[{index}].depth_bb"),
                dimension.depth_bb,
            );
        }
        errors.finish()
    }
}

fn validate_dimension_fields(
    errors: &mut ValidationErrorDetails,
    strategy_path: impl Into<String>,
    strategy: &str,
    player_count_path: impl Into<String>,
    player_count: u32,
    depth_bb_path: impl Into<String>,
    depth_bb: u32,
) {
    validate_allowed_str(
        errors,
        strategy_path,
        strategy,
        ALLOWED_STRATEGIES,
        "must be one of \"default\"",
    );
    validate_allowed_u32(
        errors,
        player_count_path,
        player_count,
        ALLOWED_PLAYER_COUNTS,
        "must be one of 6, 8, 9",
    );
    validate_allowed_u32(
        errors,
        depth_bb_path,
        depth_bb,
        ALLOWED_DEPTH_BB,
        "must be one of 100, 200, 300",
    );
}

fn validate_batch_item(errors: &mut ValidationErrorDetails, index: usize, item: &BatchQueryItem) {
    validate_positive_u32(
        errors,
        format!("requests[{index}].concrete_line_id"),
        item.concrete_line_id,
    );
    validate_required_string(
        errors,
        format!("requests[{index}].hole_cards"),
        &item.hole_cards,
    );
}

#[utoipa::path(
    post,
    path = "/range/hand-strategy",
    tag = "range",
    request_body(content = QueryRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Single hand query result.", body = crate::http::openapi::QueryResponse),
        (status = 400, description = "Invalid JSON, validation failure, or unknown hand.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Dimension or pack not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn query(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<QueryRequest>,
) -> Result<Json<ApiResponse<QueryResult>>, HttpError> {
    query_impl(state, request).await
}

async fn query_impl(
    state: AppState,
    request: QueryRequest,
) -> Result<Json<ApiResponse<QueryResult>>, HttpError> {
    let service = state.service;
    let result = run_blocking(move || {
        service.query(
            &DimensionRef::new(request.strategy, request.player_count, request.depth_bb),
            request.concrete_line_id,
            &request.hole_cards,
        )
    })
    .await?;
    Ok(Json(ApiResponse::ok(result)))
}

#[utoipa::path(
    post,
    path = "/range/hand-strategy-batch",
    tag = "range",
    request_body(content = BatchRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Per-item batch query results.", body = crate::http::openapi::BatchResponseEnvelope),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Dimension not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn batch(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<BatchRequest>,
) -> Result<Json<ApiResponse<BatchPayload>>, HttpError> {
    batch_impl(state, request).await
}

async fn batch_impl(
    state: AppState,
    request: BatchRequest,
) -> Result<Json<ApiResponse<BatchPayload>>, HttpError> {
    let service = state.service;
    let dimension = DimensionRef::new(request.strategy, request.player_count, request.depth_bb);
    let requests: Vec<_> = request
        .requests
        .into_iter()
        .map(|item| (item.concrete_line_id, item.hole_cards))
        .collect();
    let results = run_blocking(move || service.query_batch(&dimension, &requests)).await?;
    Ok(Json(ApiResponse::ok(BatchPayload { results })))
}

#[utoipa::path(
    post,
    path = "/range/hands-by-actions",
    tag = "range",
    request_body(content = HandsByActionsRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Hands matching action filters for a concrete line.", body = crate::http::openapi::HandsByActionsResponseEnvelope),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Dimension or pack not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn hands_by_actions(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<HandsByActionsRequest>,
) -> Result<Json<ApiResponse<HandsByActionsResult>>, HttpError> {
    let filters = request
        .actions
        .map(parse_action_filters)
        .transpose()
        .map_err(HttpError::bad_request)?;
    let service = state.service;
    let result = run_blocking(move || {
        service.query_hands_by_actions(
            &DimensionRef::new(request.strategy, request.player_count, request.depth_bb),
            request.concrete_line_id,
            filters,
            request.frequency,
        )
    })
    .await?;
    Ok(Json(ApiResponse::ok(result)))
}

/// Parsed action filter error kinds returned by `parse_action_filter`.
enum ActionFilterParseError {
    /// The string does not start with any recognized action name.
    UnknownAction,
    /// A no-amount action (fold/check/call) has a trailing suffix.
    UnexpectedSuffix,
    /// An amount-bearing action has an invalid numeric suffix.
    InvalidAmount,
}

impl std::fmt::Display for ActionFilterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionFilterParseError::UnknownAction => {
                write!(f, "must be one of fold, check, call, bet, raise, allin")
            }
            ActionFilterParseError::UnexpectedSuffix => {
                write!(f, "must not have a numeric suffix")
            }
            ActionFilterParseError::InvalidAmount => {
                write!(f, "must have a valid numeric suffix (e.g. bet2.5)")
            }
        }
    }
}

/// Parse a string like "raise2.5" or "call" into an ActionFilter.
fn parse_action_filter(raw: &str) -> Result<ActionFilter, ActionFilterParseError> {
    // Known action names in descending length order to avoid ambiguous prefixes
    const NAMES: &[ActionName] = &[
        ActionName::Allin,
        ActionName::Check,
        ActionName::Raise,
        ActionName::Fold,
        ActionName::Call,
        ActionName::Bet,
    ];

    for &name in NAMES {
        let prefix = name.as_str();
        if let Some(remainder) = raw.strip_prefix(prefix) {
            let amount = match name {
                ActionName::Fold | ActionName::Call | ActionName::Check => {
                    if !remainder.is_empty() {
                        return Err(ActionFilterParseError::UnexpectedSuffix);
                    }
                    None
                }
                ActionName::Bet | ActionName::Raise | ActionName::Allin => {
                    if remainder.is_empty() {
                        None
                    } else {
                        let amount: f32 = remainder
                            .parse()
                            .map_err(|_| ActionFilterParseError::InvalidAmount)?;
                        if !amount.is_finite() {
                            return Err(ActionFilterParseError::InvalidAmount);
                        }
                        Some(amount)
                    }
                }
            };
            return Ok(ActionFilter {
                raw: raw.to_owned(),
                action_name: name,
                amount_bb: amount,
            });
        }
    }

    Err(ActionFilterParseError::UnknownAction)
}

/// Parse a list of raw action filter strings.
fn parse_action_filters(raw_filters: Vec<String>) -> Result<Vec<ActionFilter>, String> {
    raw_filters
        .into_iter()
        .map(|raw| parse_action_filter(&raw).map_err(|e| e.to_string()))
        .collect()
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

#[utoipa::path(
    post,
    path = "/range/prewarm",
    tag = "range",
    request_body(content = PrewarmRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Prewarm summary.", body = crate::http::openapi::PrewarmResponseEnvelope),
        (status = 400, description = "Invalid JSON, validation failure, or unknown dimension.", body = crate::http::openapi::ErrorResponse),
        (status = 404, description = "Dimension files not found.", body = crate::http::openapi::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::openapi::ErrorResponse)
    )
)]
pub async fn prewarm(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<PrewarmRequest>,
) -> Result<Json<ApiResponse<PrewarmPayload>>, HttpError> {
    prewarm_impl(state, request).await
}

async fn prewarm_impl(
    state: AppState,
    request: PrewarmRequest,
) -> Result<Json<ApiResponse<PrewarmPayload>>, HttpError> {
    let service = state.service;
    let result = run_blocking(move || {
        for dimension in &request.dimensions {
            service.prewarm(&DimensionRef::new(
                dimension.strategy.clone(),
                dimension.player_count,
                dimension.depth_bb,
            ))?;
        }
        Ok(PrewarmPayload {
            prewarmed: request.dimensions.len(),
            total_open: service.open_handle_count(),
        })
    })
    .await?;
    Ok(Json(ApiResponse::ok(result)))
}
