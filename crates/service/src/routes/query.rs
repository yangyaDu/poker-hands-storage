use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::http::{AppState, HttpError};
use crate::naming::DimensionRef;
use crate::query_service::{BatchItemResult, QueryResult};
use crate::validation::{
    validate_positive_u32, validate_required_string, ValidateRequest, ValidatedJson,
    ValidationErrorDetails, MAX_BATCH_REQUESTS, MAX_PREWARM_DIMENSIONS,
};

use super::{run_blocking, run_infallible_blocking};

#[derive(Deserialize, ToSchema)]
pub struct QueryRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    strategy: String,
    /// Number of players for the target game tree.
    #[schema(example = 6, minimum = 1)]
    player_count: u32,
    /// Stack depth in big blinds.
    #[schema(example = 100, minimum = 1)]
    depth_bb: u32,
    /// Concrete line id from the selected dimension.
    #[schema(example = 1, minimum = 1)]
    concrete_line_id: u32,
    /// Hole cards as a 169-hand code or two-card code, for example `AA`, `AKs`, `AKo`, or `AsKh`.
    #[schema(example = "AA")]
    hole_cards: String,
}

#[derive(Deserialize, ToSchema)]
pub struct BatchRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    strategy: String,
    /// Number of players for the target game tree.
    #[schema(example = 6, minimum = 1)]
    player_count: u32,
    /// Stack depth in big blinds.
    #[schema(example = 100, minimum = 1)]
    depth_bb: u32,
    /// Query items to resolve in one dimension. Maximum 500 items per request.
    #[schema(max_items = 500)]
    requests: Vec<BatchQueryItem>,
}

#[derive(Deserialize, ToSchema)]
pub struct BatchQueryItem {
    /// Concrete line id from the selected dimension.
    #[schema(example = 1, minimum = 1)]
    concrete_line_id: u32,
    /// Hole cards as a 169-hand code or two-card code.
    #[schema(example = "AA")]
    hole_cards: String,
}

#[derive(Serialize, ToSchema)]
pub struct BatchResponse {
    /// Per-item query result. Invalid hand inputs are reported on their individual item.
    results: Vec<BatchItemResult>,
}

#[derive(Deserialize, ToSchema)]
pub struct PrewarmRequest {
    /// Dimensions to open and validate in the handle pool. Maximum 64 dimensions per request.
    #[schema(max_items = 64)]
    dimensions: Vec<DimensionRequest>,
}

#[derive(Deserialize, ToSchema)]
pub struct DimensionRequest {
    /// Strategy name. The current data set uses `default`.
    #[schema(example = "default")]
    strategy: String,
    /// Number of players for the target game tree.
    #[schema(example = 6, minimum = 1)]
    player_count: u32,
    /// Stack depth in big blinds.
    #[schema(example = 100, minimum = 1)]
    depth_bb: u32,
}

#[derive(Serialize, ToSchema)]
pub struct PrewarmResponse {
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
    validate_required_string(errors, strategy_path, strategy);
    validate_positive_u32(errors, player_count_path, player_count);
    validate_positive_u32(errors, depth_bb_path, depth_bb);
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
    path = "/query",
    tag = "query",
    request_body(content = QueryRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Single hand query result.", body = QueryResult),
        (status = 400, description = "Invalid JSON, validation failure, or unknown hand.", body = crate::http::ErrorResponse),
        (status = 404, description = "Dimension or pack not found.", body = crate::http::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::ErrorResponse)
    )
)]
pub async fn query(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<QueryRequest>,
) -> Result<Json<QueryResult>, HttpError> {
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

#[utoipa::path(
    post,
    path = "/batch",
    tag = "query",
    request_body(content = BatchRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Per-item batch query results.", body = BatchResponse),
        (status = 400, description = "Invalid JSON or validation failure.", body = crate::http::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::ErrorResponse)
    )
)]
pub async fn batch(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<BatchRequest>,
) -> Result<Json<BatchResponse>, HttpError> {
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

#[utoipa::path(
    post,
    path = "/prewarm",
    tag = "query",
    request_body(content = PrewarmRequest, content_type = "application/json"),
    responses(
        (status = 200, description = "Prewarm summary.", body = PrewarmResponse),
        (status = 400, description = "Invalid JSON, validation failure, or unknown dimension.", body = crate::http::ErrorResponse),
        (status = 404, description = "Dimension files not found.", body = crate::http::ErrorResponse),
        (status = 500, description = "Internal service error.", body = crate::http::ErrorResponse)
    )
)]
pub async fn prewarm(
    State(state): State<AppState>,
    ValidatedJson(request): ValidatedJson<PrewarmRequest>,
) -> Result<Json<PrewarmResponse>, HttpError> {
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
