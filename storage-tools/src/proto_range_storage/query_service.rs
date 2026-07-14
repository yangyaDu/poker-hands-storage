use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use range_store_core::dimension::DimensionRef;
use range_store_core::hole_cards::{hand_code_from_id, parse_hole_cards};
use range_store_core::query::{
    parse_action_filters, ActionFilter, ActionResult, FrequencyFilter, QueryBatchItemResult,
    QueryBatchResult, QueryResult,
};

use crate::errors::ToolError;

use super::line_matrix_store::{
    CompactArchiveOpenOptions, CompactLineMatrixArchive, DecodedCompactLineMatrix,
    ProfiledMatrixRead,
};
use super::proto::ActionType;

pub struct ProtoRangeQueryService {
    store: CompactLineMatrixArchive,
}

#[derive(Debug, Clone)]
pub struct HandStrategyPhaseProfile {
    pub dimension_check_ms: f64,
    pub parse_hand_ms: f64,
    pub matrix_read_ms: f64,
    pub matrix_cache_hit: bool,
    pub matrix_cache_lookup_ms: f64,
    pub matrix_index_payload_ms: f64,
    pub matrix_protobuf_decode_ms: f64,
    pub matrix_compact_index_ms: f64,
    pub matrix_cache_insert_ms: f64,
    pub action_materialization_ms: f64,
    pub service_total_ms: f64,
}

#[derive(Debug, Clone)]
pub struct ProfiledHandStrategyResult {
    pub result: QueryResult,
    pub profile: HandStrategyPhaseProfile,
}

impl ProtoRangeQueryService {
    pub fn open(archive_dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(archive_dir, CompactArchiveOpenOptions::default())
    }

    pub fn open_with_options(
        archive_dir: impl AsRef<Path>,
        options: CompactArchiveOpenOptions,
    ) -> Result<Self, ToolError> {
        Ok(Self {
            store: CompactLineMatrixArchive::open_with_options(archive_dir.as_ref(), options)?,
        })
    }

    pub fn query_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        Ok(self
            .profile_hand_strategy(dimension, concrete_line_id, hole_cards)?
            .result)
    }

    pub fn profile_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<ProfiledHandStrategyResult, ToolError> {
        let total_started = Instant::now();
        let stage_started = Instant::now();
        self.require_dimension(dimension)?;
        let dimension_check_ms = elapsed_ms(stage_started);
        let stage_started = Instant::now();
        let hand = parse_hole_cards(hole_cards)?;
        let parse_hand_ms = elapsed_ms(stage_started);
        let stage_started = Instant::now();
        let matrix = self.read_matrix_profiled(dimension, concrete_line_id)?;
        let matrix_read_ms = elapsed_ms(stage_started);
        let stage_started = Instant::now();
        let actions = self.actions_for_hand(
            &matrix.matrix,
            usize::from(hand.hand_id),
            &hand.hand_code,
            dimension,
            concrete_line_id,
        )?;
        Ok(ProfiledHandStrategyResult {
            result: QueryResult { actions },
            profile: HandStrategyPhaseProfile {
                dimension_check_ms,
                parse_hand_ms,
                matrix_read_ms,
                matrix_cache_hit: matrix.profile.cache_hit,
                matrix_cache_lookup_ms: matrix.profile.cache_lookup_ms,
                matrix_index_payload_ms: matrix.profile.index_payload_ms,
                matrix_protobuf_decode_ms: matrix.profile.protobuf_decode_ms,
                matrix_compact_index_ms: matrix.profile.compact_index_ms,
                matrix_cache_insert_ms: matrix.profile.cache_insert_ms,
                action_materialization_ms: elapsed_ms(stage_started),
                service_total_ms: elapsed_ms(total_started),
            },
        })
    }

    pub fn matrix_count(&self, dimension: &DimensionRef) -> Result<u64, ToolError> {
        self.require_dimension(dimension)?;
        Ok(self.store.matrix_count())
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<QueryBatchResult, ToolError> {
        if requests.is_empty() {
            return Ok(QueryBatchResult { results: vec![] });
        }

        let dimension_error = self.require_dimension(dimension).err();
        let dimension_is_available = dimension_error.is_none();
        let mut first_failure = None;
        let mut parsed_hands = Vec::with_capacity(requests.len());
        for (index, (_concrete_line_id, hole_cards)) in requests.iter().enumerate() {
            match parse_hole_cards(hole_cards) {
                Ok(hand) => parsed_hands.push(Some(hand)),
                Err(error) => {
                    record_first_failure(&mut first_failure, index, ToolError::from(error));
                    parsed_hands.push(None);
                }
            }
        }
        if let Some(error) = dimension_error {
            record_first_failure(&mut first_failure, 0, error);
        }

        let mut groups: HashMap<u32, Vec<(usize, u8)>> = HashMap::new();
        for (index, ((concrete_line_id, _hole_cards), hand)) in
            requests.iter().zip(parsed_hands.iter()).enumerate()
        {
            let Some(hand) = hand else {
                continue;
            };
            groups
                .entry(*concrete_line_id)
                .or_default()
                .push((index, hand.hand_id));
        }

        let mut actions_by_request = vec![None; requests.len()];
        if dimension_is_available {
            for (concrete_line_id, group) in groups {
                let group_min_index = group[0].0;
                let matrix = match self.read_matrix(dimension, concrete_line_id) {
                    Ok(matrix) => matrix,
                    Err(error) => {
                        record_first_failure(&mut first_failure, group_min_index, error);
                        continue;
                    }
                };
                for (request_index, hand_id) in group {
                    let Some(hand) = parsed_hands[request_index].as_ref() else {
                        continue;
                    };
                    match self.actions_for_hand(
                        &matrix,
                        usize::from(hand_id),
                        &hand.hand_code,
                        dimension,
                        concrete_line_id,
                    ) {
                        Ok(actions) => actions_by_request[request_index] = Some(actions),
                        Err(error) => {
                            record_first_failure(&mut first_failure, request_index, error)
                        }
                    }
                }
            }
        }

        if let Some((index, error)) = first_failure {
            let (concrete_line_id, hole_cards) = &requests[index];
            return Err(ToolError::new(
                "BATCH_ITEM_ERROR",
                format!(
                    "requests[{index}] concrete_line_id={concrete_line_id}, hole_cards={hole_cards:?} failed with {}: {}",
                    error.code(),
                    error.message()
                ),
            ));
        }

        let results = requests
            .iter()
            .enumerate()
            .map(
                |(index, (concrete_line_id, hole_cards))| QueryBatchItemResult {
                    concrete_line_id: *concrete_line_id,
                    hole_cards: hole_cards.clone(),
                    actions: actions_by_request[index]
                        .take()
                        .expect("every successful batch request must have actions"),
                },
            )
            .collect();
        Ok(QueryBatchResult { results })
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        self.require_dimension(dimension)?;
        let matrix = self.read_matrix(dimension, concrete_line_id)?;
        let frequency_filter = FrequencyFilter::from_request(frequency);
        let mut hands = Vec::new();

        for hand_id in 0..169 {
            let actions = self.action_values_for_hand(&matrix, hand_id)?;
            let matches = actions.iter().any(|action| {
                frequency_filter.matches(action.frequency)
                    && (action_filters.is_empty()
                        || action_filters
                            .iter()
                            .any(|filter| action_matches_filter(action, filter)))
            });
            if matches {
                hands.push(hand_code_from_id(hand_id as u8));
            }
        }
        Ok(hands)
    }

    pub fn query_hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        let filters = parse_action_filters(action_names.to_vec())
            .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
        self.query_hands_by_actions(dimension, concrete_line_id, &filters, frequency)
    }

    fn read_matrix(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
    ) -> Result<Arc<DecodedCompactLineMatrix>, ToolError> {
        self.store
            .read_matrix(u64::from(concrete_line_id))
            .map_err(|error| map_line_error(error, dimension, concrete_line_id))
    }

    fn read_matrix_profiled(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
    ) -> Result<ProfiledMatrixRead, ToolError> {
        self.store
            .read_matrix_profiled(u64::from(concrete_line_id))
            .map_err(|error| map_line_error(error, dimension, concrete_line_id))
    }

    fn actions_for_hand(
        &self,
        matrix: &DecodedCompactLineMatrix,
        hand_id: usize,
        hand_code: &str,
        dimension: &DimensionRef,
        concrete_line_id: u32,
    ) -> Result<Vec<ActionResult>, ToolError> {
        let actions = self.action_values_for_hand(matrix, hand_id)?;
        if actions.is_empty() {
            return Err(ToolError::new(
                "HAND_STRATEGY_NOT_FOUND",
                format!(
                    "Hand {} is outside the retained Proto range for concrete_line_id={concrete_line_id} in dimension {}:{}:{}",
                    hand_code, dimension.strategy, dimension.player_count, dimension.depth_bb
                ),
            ));
        }
        Ok(actions)
    }

    fn action_values_for_hand(
        &self,
        matrix: &DecodedCompactLineMatrix,
        hand_id: usize,
    ) -> Result<Vec<ActionResult>, ToolError> {
        let mut actions = Vec::new();
        for (action_index, action) in matrix.matrix().actions.iter().enumerate() {
            let Some(value) = matrix.action_value(action_index, hand_id) else {
                continue;
            };
            actions.push(ActionResult {
                action_name: action_name(action.action_type)?.to_owned(),
                action_size: action.action_size_x10000 as f32 / 10_000.0,
                amount_bb: action.amount_centi_bb as f32 / 100.0,
                frequency: f64::from(value.frequency_x10000) / 10_000.0,
                hand_ev: Some(f64::from(value.ev_x10000) / 10_000.0),
            });
        }
        Ok(actions)
    }

    fn require_dimension(&self, dimension: &DimensionRef) -> Result<(), ToolError> {
        let stored = self.store.dimension();
        if stored.strategy == dimension.strategy
            && stored.player_count == dimension.player_count
            && stored.depth_bb == dimension.depth_bb
        {
            return Ok(());
        }
        Err(ToolError::new(
            "DIMENSION_NOT_FOUND",
            format!(
                "Proto archive contains {}:{}:{}, not {}:{}:{}",
                stored.strategy,
                stored.player_count,
                stored.depth_bb,
                dimension.strategy,
                dimension.player_count,
                dimension.depth_bb
            ),
        ))
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn action_matches_filter(action: &ActionResult, filter: &ActionFilter) -> bool {
    action.action_name == filter.action_name.as_str()
        && match filter.amount_bb {
            Some(amount_bb) => (action.amount_bb - amount_bb).abs() <= f32::EPSILON,
            None => true,
        }
}

fn record_first_failure(
    first_failure: &mut Option<(usize, ToolError)>,
    index: usize,
    error: ToolError,
) {
    if first_failure
        .as_ref()
        .is_none_or(|(first_index, _)| index < *first_index)
    {
        *first_failure = Some((index, error));
    }
}

fn map_line_error(error: ToolError, dimension: &DimensionRef, concrete_line_id: u32) -> ToolError {
    if error.code() == "LINE_NOT_FOUND" {
        return ToolError::new(
            "CONCRETE_LINE_NOT_FOUND",
            format!(
                "Concrete line {concrete_line_id} is not in Proto dimension {}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ),
        );
    }
    error
}

fn action_name(raw_action_type: i32) -> Result<&'static str, ToolError> {
    match ActionType::try_from(raw_action_type) {
        Ok(ActionType::Fold) => Ok("fold"),
        Ok(ActionType::Check) => Ok("check"),
        Ok(ActionType::Call) => Ok("call"),
        Ok(ActionType::Bet) => Ok("bet"),
        Ok(ActionType::Raise) => Ok("raise"),
        Ok(ActionType::Allin) => Ok("allin"),
        _ => Err(ToolError::invalid_format(format!(
            "Unsupported Proto action_type {raw_action_type}"
        ))),
    }
}
