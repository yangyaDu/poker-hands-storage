use std::path::Path;

use range_store_core::dimension::DimensionRef;
use range_store_core::hole_cards::{hand_code_from_id, parse_hole_cards};
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::query::{
    parse_action_filters, ActionFilter, ActionResult, FrequencyFilter, QueryBatchItemResult,
    QueryBatchResult, QueryResult,
};

use crate::errors::ToolError;

use super::archive::{V3Archive, V3ArchiveOpenOptions};
use super::cache::ByteCacheStats;
use super::proto::ActionType;
use super::strategy_codec::DecodedHandStrategy;

pub struct V3QueryService {
    archive: V3Archive,
}

impl V3QueryService {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(dir, V3ArchiveOpenOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: V3ArchiveOpenOptions,
    ) -> Result<Self, ToolError> {
        Ok(Self {
            archive: V3Archive::open_with_options(dir, options)?,
        })
    }

    pub fn query_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_action_path_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        self.require_dimension(dimension)?;
        let hand = parse_hole_cards(hole_cards)?;
        let strategy = self.archive.strategies().read(concrete_action_path_id)?;
        let actions = actions_for_hand(&strategy, usize::from(hand.hand_id))?;
        if actions.is_empty() {
            return Err(ToolError::new(
                "HAND_STRATEGY_NOT_FOUND",
                format!(
                    "Hand {} is not available for V3 concrete action path id {concrete_action_path_id}",
                    hand.hand_code
                ),
            ));
        }
        Ok(QueryResult { actions })
    }

    pub fn query_hand_strategy_by_path(
        &self,
        dimension: &DimensionRef,
        concrete_action_path: &str,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        self.require_dimension(dimension)?;
        let id = self
            .archive
            .metadata()
            .resolve_concrete_action_path(concrete_action_path)?;
        self.query_hand_strategy(dimension, id, hole_cards)
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<QueryBatchResult, ToolError> {
        let mut results = Vec::with_capacity(requests.len());
        for (index, (concrete_action_path_id, hole_cards)) in requests.iter().enumerate() {
            let result = self
                .query_hand_strategy(dimension, *concrete_action_path_id, hole_cards)
                .map_err(|error| {
                    ToolError::new(
                        "BATCH_ITEM_ERROR",
                        format!(
                            "requests[{index}] concrete_line_id={concrete_action_path_id}, hole_cards={hole_cards:?} failed with {}: {}",
                            error.code(),
                            error.message()
                        ),
                    )
                })?;
            results.push(QueryBatchItemResult {
                concrete_line_id: *concrete_action_path_id,
                hole_cards: hole_cards.clone(),
                actions: result.actions,
            });
        }
        Ok(QueryBatchResult { results })
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_action_path_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        self.require_dimension(dimension)?;
        let strategy = self.archive.strategies().read(concrete_action_path_id)?;
        let frequency_filter = FrequencyFilter::from_request(frequency);
        let mut hands = Vec::new();
        for hand_id in 0..169 {
            let actions = actions_for_hand(&strategy, hand_id)?;
            if actions.iter().any(|action| {
                frequency_filter.matches(action.frequency)
                    && (action_filters.is_empty()
                        || action_filters
                            .iter()
                            .any(|filter| action_matches_filter(action, filter)))
            }) {
                hands.push(hand_code_from_id(hand_id as u8));
            }
        }
        Ok(hands)
    }

    pub fn query_hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_action_path_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        let filters = parse_action_filters(action_names.to_vec())
            .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
        self.query_hands_by_actions(dimension, concrete_action_path_id, &filters, frequency)
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, ToolError> {
        self.require_dimension(dimension)?;
        self.archive.metadata().get_concrete_lines(filter)
    }

    pub fn get_drill_scenario_lines(
        &self,
        dimension: &DimensionRef,
        drill_name: &str,
    ) -> Result<Vec<String>, ToolError> {
        self.require_dimension(dimension)?;
        self.archive.metadata().get_drill_scenario_lines(drill_name)
    }

    pub fn metadata_cache_stats(&self) -> ByteCacheStats {
        self.archive.metadata().cache_stats()
    }

    pub fn strategy_cache_stats(&self) -> ByteCacheStats {
        self.archive.strategies().cache_stats()
    }

    pub fn record_count(&self) -> u64 {
        self.archive.strategies().record_count()
    }

    pub(crate) fn resize_cache_budgets(
        &self,
        metadata_cache_byte_budget: usize,
        strategy_cache_byte_budget: usize,
    ) {
        self.archive
            .resize_cache_budgets(metadata_cache_byte_budget, strategy_cache_byte_budget);
    }

    pub(crate) fn cache_budgets(&self) -> (usize, usize) {
        self.archive.cache_budgets()
    }

    fn require_dimension(&self, requested: &DimensionRef) -> Result<(), ToolError> {
        let stored = self.archive.manifest();
        if stored.strategy == requested.strategy
            && stored.player_count == requested.player_count
            && stored.depth_bb == requested.depth_bb
        {
            return Ok(());
        }
        Err(ToolError::new(
            "DIMENSION_NOT_FOUND",
            format!(
                "V3 archive contains {}:{}:{}, not {}:{}:{}",
                stored.strategy,
                stored.player_count,
                stored.depth_bb,
                requested.strategy,
                requested.player_count,
                requested.depth_bb
            ),
        ))
    }
}

fn actions_for_hand(
    strategy: &DecodedHandStrategy,
    hand_id: usize,
) -> Result<Vec<ActionResult>, ToolError> {
    let mut actions = Vec::new();
    for (action_index, action) in strategy.strategy().actions.iter().enumerate() {
        let Some(value) = strategy.action_value(action_index, hand_id) else {
            continue;
        };
        actions.push(ActionResult {
            action_name: action_name(action.action_type)?.to_owned(),
            action_size: action.action_size_x10000 as f32 / 10_000.0,
            amount_bb: action.amount_centi_bb as f32 / 100.0,
            frequency: f64::from(value.frequency_x10000) / 10_000.0,
            hand_ev: if value.hand_ev_is_null {
                None
            } else {
                Some(f64::from(value.hand_ev_x10000) / 10_000.0)
            },
        });
    }
    Ok(actions)
}

fn action_matches_filter(action: &ActionResult, filter: &ActionFilter) -> bool {
    action.action_name == filter.action_name.as_str()
        && match filter.amount_bb {
            Some(amount_bb) => (action.amount_bb - amount_bb).abs() <= f32::EPSILON,
            None => true,
        }
}

fn action_name(raw_action_type: i32) -> Result<&'static str, ToolError> {
    match ActionType::try_from(raw_action_type) {
        Ok(ActionType::Fold) => Ok("fold"),
        Ok(ActionType::Check) => Ok("check"),
        Ok(ActionType::Call) => Ok("call"),
        Ok(ActionType::Bet) => Ok("bet"),
        Ok(ActionType::Raise) => Ok("raise"),
        Ok(ActionType::Allin) => Ok("allin"),
        _ => Err(ToolError::new(
            "INVALID_V3_HAND_STRATEGY",
            format!("Unsupported V3 action_type {raw_action_type}"),
        )),
    }
}
