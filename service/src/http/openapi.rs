use utoipa::OpenApi;

use crate::query::{BatchItemResult, HandsByActionsResult, QueryResult};
use crate::routes::hand_query_routes::{
    BatchQueryItem, BatchRequest, HandsByActionsRequest, PrewarmRequest, QueryRequest,
};
use crate::routes::health_routes::{HealthResponse, ReadyResponse};
use crate::routes::metadata_routes::{ConcreteLinesRequest, DrillScenarioLinesRequest};
use range_store_core::metadata::ConcreteLineRow;

/// Concrete response types for OpenAPI documentation.
/// utoipa doesn't support generic `ApiResponse<T>` in path macros,
/// so we define concrete wrappers for each endpoint.
/// These types are only used for OpenAPI schema generation.
#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct QueryResponse {
    code: i32,
    data: QueryResult,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct BatchResponseEnvelope {
    code: i32,
    data: BatchData,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct BatchData {
    results: Vec<BatchItemResult>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct HandsByActionsResponseEnvelope {
    code: i32,
    data: HandsByActionsResult,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct PrewarmResponseEnvelope {
    code: i32,
    data: PrewarmData,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct PrewarmData {
    prewarmed: usize,
    total_open: usize,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct ConcreteLinesResponseEnvelope {
    code: i32,
    data: ConcreteLinesPayload,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct ConcreteLinesPayload {
    pub lines: Vec<ConcreteLineRow>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct DrillScenarioLinesResponseEnvelope {
    code: i32,
    data: DrillScenarioLinesPayload,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct DrillScenarioLinesPayload {
    pub abstract_lines: Vec<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct HealthResponseEnvelope {
    code: i32,
    data: HealthResponse,
    #[schema(nullable)]
    message: Option<String>,
}

#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct ReadyResponseEnvelope {
    code: i32,
    data: ReadyResponse,
    #[schema(nullable)]
    message: Option<String>,
}

/// Error response body (used for 4xx/5xx responses).
#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
pub struct ErrorResponse {
    code: i32,
    #[schema(nullable)]
    data: Option<()>,
    message: String,
}

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
        BatchItemResult,
        BatchQueryItem,
        BatchRequest,
        BatchData,
        BatchResponseEnvelope,
        ConcreteLineRow,
        ConcreteLinesPayload,
        ConcreteLinesRequest,
        ConcreteLinesResponseEnvelope,
        DrillScenarioLinesPayload,
        DrillScenarioLinesRequest,
        DrillScenarioLinesResponseEnvelope,
        ErrorResponse,
        HandsByActionsRequest,
        HandsByActionsResult,
        HandsByActionsResponseEnvelope,
        HealthResponse,
        HealthResponseEnvelope,
        PrewarmRequest,
        PrewarmData,
        PrewarmResponseEnvelope,
        QueryRequest,
        QueryResponse,
        ReadyResponse,
        ReadyResponseEnvelope
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
