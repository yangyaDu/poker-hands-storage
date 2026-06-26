use utoipa::OpenApi;

use crate::http::request_validation::{FieldValidationError, ValidationErrorDetails};
use crate::http::ErrorResponse;
use crate::query::{ActionResult, BatchItemResult, BatchStrategyResult, ErrorInfo, QueryResult};
use crate::routes::hand_query_routes::{
    BatchQueryItem, BatchRequest, BatchResponse, DimensionRequest, PrewarmRequest, PrewarmResponse,
    QueryRequest,
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
        crate::routes::hand_query_routes::prewarm,
        crate::routes::metadata_routes::concrete_lines,
        crate::routes::metadata_routes::drill_scenario_lines
    ),
    components(schemas(
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
        (name = "query", description = "Preflop range query endpoints"),
        (name = "metadata", description = "Concrete line and drill scenario metadata endpoints")
    ),
    info(
        title = "Poker Hands Storage API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Read-only query service for PFSP preflop range storage data."
    )
)]
pub struct ApiDoc;
