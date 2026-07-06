use std::path::PathBuf;

use range_store_core::dimension::DimensionRef;
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::query::{
    ActionFilter, BatchItemResult as CoreBatchItemResult, QueryResult as CoreQueryResult,
    RangeStoreFacade,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::AppError;

pub struct QueryService {
    facade: RangeStoreFacade,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct ActionResult {
    /// Action name, for example `fold`, `call`, or `raise`.
    pub action_name: String,
    /// Abstract action size from the source range data.
    pub action_size: f32,
    /// Amount in big blinds.
    pub amount_bb: f32,
    /// Strategy frequency for this hand/action.
    pub frequency: f64,
    /// Optional expected value for this hand/action.
    pub hand_ev: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq)]
pub struct QueryResult {
    /// Original hole-card input from the request.
    pub input_hole_cards: String,
    /// Normalized 169-hand code.
    pub hand_code: String,
    /// Ordered action strategy entries.
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchItemResult {
    /// Concrete line id for this batch item.
    pub concrete_line_id: u32,
    /// Original hole-card input for this batch item.
    pub input_hole_cards: String,
    /// Normalized 169-hand code when the hand input is valid.
    pub hand_code: Option<String>,
    /// Strategy result when the item was resolved successfully.
    pub strategy: Option<BatchStrategyResult>,
    /// Per-item error for invalid hand input or lookup failures.
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BatchStrategyResult {
    /// Ordered action strategy entries.
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ErrorInfo {
    /// Public API error code: 1000, 404, or 500.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HandsByActionsResult {
    /// Matching 169-hand codes.
    pub hands: Vec<String>,
}

impl QueryService {
    pub fn open(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, AppError> {
        Ok(Self {
            facade: RangeStoreFacade::open(data_dir, max_open_handles, verify_checksums)?,
        })
    }

    pub fn open_with_meta(
        data_dir: impl Into<PathBuf>,
        meta_path: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, AppError> {
        Ok(Self {
            facade: RangeStoreFacade::open_with_meta(
                data_dir,
                meta_path,
                max_open_handles,
                verify_checksums,
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
            .map_err(AppError::from)
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<Vec<BatchItemResult>, AppError> {
        Ok(self
            .facade
            .query_batch(dimension, requests)?
            .into_iter()
            .map(batch_item_from_core)
            .collect())
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<usize, AppError> {
        self.facade.prewarm(dimension).map_err(AppError::from)
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, AppError> {
        self.facade
            .get_concrete_lines(dimension, filter)
            .map_err(AppError::from)
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, AppError> {
        self.facade
            .get_drill_scenario_lines(strategy, drill_name, player_count, drill_depth)
            .map_err(AppError::from)
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: Option<Vec<ActionFilter>>,
        frequency: Option<f64>,
    ) -> Result<HandsByActionsResult, AppError> {
        let filters = action_filters.unwrap_or_default();
        self.facade
            .hands_by_actions(dimension, concrete_line_id, &filters, frequency)
            .map(|hands| HandsByActionsResult { hands })
            .map_err(AppError::from)
    }

    pub fn schema_count(&self) -> usize {
        self.facade.schema_count()
    }

    pub fn open_handle_count(&self) -> usize {
        self.facade.open_handle_count()
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        self.facade.known_dimensions()
    }
}

fn query_result_from_core(result: CoreQueryResult) -> QueryResult {
    QueryResult {
        input_hole_cards: result.input_hole_cards,
        hand_code: result.hand_code,
        actions: result.actions.into_iter().map(action_from_core).collect(),
    }
}

fn batch_item_from_core(item: CoreBatchItemResult) -> BatchItemResult {
    BatchItemResult {
        concrete_line_id: item.concrete_line_id,
        input_hole_cards: item.hole_cards,
        hand_code: item.hand_code,
        strategy: item.actions.map(|actions| BatchStrategyResult {
            actions: actions.into_iter().map(action_from_core).collect(),
        }),
        error: item.error.map(|error| ErrorInfo {
            code: error.code,
            message: error.message,
        }),
    }
}

fn action_from_core(action: range_store_core::query::ActionResult) -> ActionResult {
    ActionResult {
        action_name: action.action_name,
        action_size: action.action_size,
        amount_bb: action.amount_bb,
        frequency: action.frequency,
        hand_ev: action.hand_ev,
    }
}
