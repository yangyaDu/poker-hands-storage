use std::path::PathBuf;

use poker_hands_storage_tools::errors::ToolError;
use poker_hands_storage_tools::proto_range_storage::v3::facade::{V3Facade, V3FacadeOptions};
use range_store_core::dimension::DimensionRef;
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::query::{
    ActionFilter, ActionResult as CoreActionResult, FrequencyFilter, QueryResult as CoreQueryResult,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::AppError;

pub struct QueryService {
    facade: V3Facade,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct ActionResult {
    pub action_name: String,
    pub action_size: f32,
    pub amount_bb: f32,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct QueryResult {
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchItemResult {
    pub concrete_line_id: u32,
    pub hole_cards: String,
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HandsByActionsResult {
    pub hands: Vec<String>,
}

impl QueryService {
    pub fn open(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, AppError> {
        Self::open_with_options(
            data_dir,
            max_open_handles,
            verify_checksums,
            8 * 1024 * 1024,
            64 * 1024 * 1024,
        )
    }

    pub fn open_with_options(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
        metadata_cache_bytes_per_handle: usize,
        strategy_cache_bytes_per_handle: usize,
    ) -> Result<Self, AppError> {
        Ok(Self {
            facade: V3Facade::open_with_options(
                data_dir.into(),
                V3FacadeOptions {
                    max_open_handles,
                    verify_file_checksums: verify_checksums,
                    metadata_cache_byte_budget_per_handle: metadata_cache_bytes_per_handle,
                    strategy_cache_byte_budget_per_handle: strategy_cache_bytes_per_handle,
                },
            )?,
        })
    }

    pub fn query(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, AppError> {
        self.facade
            .query_hand_strategy(dimension, concrete_line_id, hole_cards)
            .map(query_result_from_core)
            .map_err(|error| map_query_error(error, dimension, concrete_line_id, hole_cards))
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<Vec<BatchItemResult>, AppError> {
        requests
            .iter()
            .enumerate()
            .map(|(index, (concrete_line_id, hole_cards))| {
                let result = self
                    .query(dimension, *concrete_line_id, hole_cards)
                    .map_err(|error| {
                        AppError::new(
                            error.code(),
                            format!(
                                "Batch item requests[{index}] failed: {} from concrete_line_id={} dimension={}:{}:{}",
                                error.message(),
                                concrete_line_id,
                                dimension.strategy,
                                dimension.player_count,
                                dimension.depth_bb
                            ),
                        )
                    })?;
                Ok(BatchItemResult {
                    concrete_line_id: *concrete_line_id,
                    hole_cards: hole_cards.clone(),
                    actions: result.actions,
                })
            })
            .collect()
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<usize, AppError> {
        self.facade.prewarm(dimension)?;
        Ok(1)
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, AppError> {
        self.facade
            .get_concrete_lines(dimension, filter.clone())
            .map_err(|error| match error.code() {
                "CONCRETE_LINE_NOT_FOUND" => match filter {
                    ConcreteLineFilter::Abstract(abstract_line) => {
                        AppError::abstract_line_not_found(
                            &dimension.strategy,
                            dimension.player_count,
                            dimension.depth_bb,
                            abstract_line,
                        )
                    }
                    ConcreteLineFilter::Concrete(concrete_line) => {
                        AppError::concrete_line_value_not_found(
                            &dimension.strategy,
                            dimension.player_count,
                            dimension.depth_bb,
                            concrete_line,
                        )
                    }
                    ConcreteLineFilter::AbstractAndConcrete {
                        abstract_line,
                        concrete_line,
                    } => AppError::concrete_line_filter_not_found(
                        &dimension.strategy,
                        dimension.player_count,
                        dimension.depth_bb,
                        abstract_line,
                        concrete_line,
                    ),
                },
                _ => error.into(),
            })
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, AppError> {
        let dimension = DimensionRef::new(strategy, player_count, drill_depth);
        self.facade
            .get_drill_scenario_lines(strategy, drill_name, player_count, drill_depth)
            .map_err(|error| match error.code() {
                "DRILL_SCENARIO_NOT_FOUND" => AppError::drill_scenario_not_found(
                    strategy,
                    drill_name,
                    player_count,
                    drill_depth,
                ),
                "DIMENSION_NOT_FOUND" => AppError::dimension_not_found(
                    &dimension.strategy,
                    dimension.player_count,
                    dimension.depth_bb,
                ),
                _ => error.into(),
            })
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: Option<Vec<ActionFilter>>,
        frequency: Option<f64>,
    ) -> Result<HandsByActionsResult, AppError> {
        let filters = action_filters.unwrap_or_default();
        let hands = self
            .facade
            .query_hands_by_actions(dimension, concrete_line_id, &filters, frequency)
            .map_err(|error| map_query_error(error, dimension, concrete_line_id, ""))?;
        if hands.is_empty() {
            let actions = if filters.is_empty() {
                "any".to_owned()
            } else {
                filters
                    .iter()
                    .map(|filter| filter.raw.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            return Err(AppError::no_hands_found(
                &actions,
                &FrequencyFilter::from_request(frequency).description(),
                concrete_line_id,
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            ));
        }
        Ok(HandsByActionsResult { hands })
    }

    pub fn schema_count(&self) -> usize {
        self.facade.known_dimensions().len()
    }

    pub fn open_handle_count(&self) -> usize {
        self.facade.cache_stats().open_handle_count
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        self.facade.known_dimensions()
    }
}

fn map_query_error(
    error: ToolError,
    dimension: &DimensionRef,
    concrete_line_id: u32,
    hole_cards: &str,
) -> AppError {
    match error.code() {
        "UNKNOWN_HAND" => AppError::invalid_argument(error.message()),
        "DIMENSION_NOT_FOUND" => AppError::dimension_not_found(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        ),
        "CONCRETE_LINE_NOT_FOUND" => AppError::concrete_line_not_found(
            concrete_line_id,
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        ),
        "HAND_STRATEGY_NOT_FOUND" => AppError::hand_outside_action_line(
            hole_cards,
            concrete_line_id,
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        ),
        _ => error.into(),
    }
}

fn query_result_from_core(result: CoreQueryResult) -> QueryResult {
    QueryResult {
        actions: result.actions.into_iter().map(action_from_core).collect(),
    }
}

fn action_from_core(action: CoreActionResult) -> ActionResult {
    ActionResult {
        action_name: action.action_name,
        action_size: action.action_size,
        amount_bb: action.amount_bb,
        frequency: action.frequency,
        hand_ev: action.hand_ev,
    }
}
