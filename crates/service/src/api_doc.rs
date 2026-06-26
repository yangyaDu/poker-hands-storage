use utoipa::OpenApi;

use crate::http::ErrorResponse;
use crate::meta_db::ConcreteLineRow;
use crate::query_service::{
    ActionResult, BatchItemResult, BatchStrategyResult, ErrorInfo, QueryResult,
};
use crate::routes::health::{HealthResponse, ReadyResponse};
use crate::routes::metadata::{
    ConcreteLinesRequest, ConcreteLinesResponse, DrillScenarioLinesRequest,
    DrillScenarioLinesResponse,
};
use crate::routes::query::{
    BatchQueryItem, BatchRequest, BatchResponse, DimensionRequest, PrewarmRequest, PrewarmResponse,
    QueryRequest,
};
use crate::validation::{FieldValidationError, ValidationErrorDetails};

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::routes::health::health,
        crate::routes::health::ready,
        crate::routes::query::query,
        crate::routes::query::batch,
        crate::routes::query::prewarm,
        crate::routes::metadata::concrete_lines,
        crate::routes::metadata::drill_scenario_lines
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
