pub mod dimension_handle_pool;
pub mod hand_query_service;

pub use hand_query_service::{
    ActionHandsEntry, ActionResult, BatchItemResult, BatchStrategyResult, ErrorInfo,
    HandsByActionsResult, QueryResult, QueryService,
};
