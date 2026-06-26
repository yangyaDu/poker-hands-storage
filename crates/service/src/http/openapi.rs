use utoipa::OpenApi;

use crate::http::request_validation::{FieldValidationError, ValidationErrorDetails};
use crate::http::ErrorResponse;
use crate::query::{
    ActionHandsEntry, ActionResult, BatchItemResult, BatchStrategyResult, ErrorInfo,
    HandsByActionsResult, QueryResult,
};
use crate::routes::hand_query_routes::{
    BatchQueryItem, BatchRequest, BatchResponse, DimensionRequest, HandsByActionsRequest,
    PrewarmRequest, PrewarmResponse, QueryRequest,
};
use crate::routes::health_routes::{HealthResponse, ReadyResponse};
use crate::routes::metadata_routes::{
    ConcreteLinesRequest, ConcreteLinesResponse, DrillScenarioLinesRequest,
    DrillScenarioLinesResponse,
};
use crate::storage::metadata::ConcreteLineRow;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::health_routes::health,
        crate::routes::health_routes::ready,
        crate::routes::hand_query_routes::query,
        crate::routes::hand_query_routes::batch,
        crate::routes::hand_query_routes::hands_by_actions,
        crate::routes::hand_query_routes::prewarm,
        crate::routes::metadata_routes::concrete_lines,
        crate::routes::metadata_routes::drill_scenario_lines
    ),
    components(schemas(
        ActionHandsEntry,
        ActionResult,
        BatchItemResult,
        BatchQueryItem,
        BatchRequest,
        BatchResponse,
        BatchStrategyResult,
        ConcreteLineRow,
        ConcreteLinesRequest,
        ConcreteLinesResponse,
        DimensionRequest,
        DrillScenarioLinesRequest,
        DrillScenarioLinesResponse,
        ErrorInfo,
        ErrorResponse,
        FieldValidationError,
        HandsByActionsRequest,
        HandsByActionsResult,
        HealthResponse,
        PrewarmRequest,
        PrewarmResponse,
        QueryRequest,
        QueryResult,
        ReadyResponse,
        ValidationErrorDetails
    )),
    tags(
        (name = "health", description = "Service health and readiness endpoints"),
        (name = "range", description = "Preflop range query and metadata endpoints")
    ),
    info(
        title = "Poker Hands Storage API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Read-only query service for PFSP preflop range storage data."
    )
)]
pub struct ApiDoc;
