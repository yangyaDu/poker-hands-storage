use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use range_store_core::DimensionReader;
use serde::Serialize;
use utoipa::ToSchema;

use crate::domain::action_schema::{ActionDef, ActionName};
use crate::domain::dimension::DimensionRef;
use crate::domain::hole_cards::{hand_code_from_id, parse_hole_cards, ParsedHand};
use crate::errors::AppError;
use crate::query::dimension_handle_pool::HandlePool;
use crate::storage::manifest::{load_manifest, queryable_dimensions};
use crate::storage::metadata::{ConcreteLineRow, MetadataReader};

/// Parsed action filter for the hands-by-actions endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionFilter {
    pub raw: String,
    pub action_name: ActionName,
    pub amount_bb: Option<f32>,
}

pub struct QueryService {
    action_schemas: HashMap<u32, Vec<ActionDef>>,
    metadata: MetadataReader,
    pool: HandlePool,
    verify_checksums: bool,
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
        let data_dir = data_dir.into();
        let meta_path = data_dir.join("meta.db");
        Self::open_with_meta(data_dir, meta_path, max_open_handles, verify_checksums)
    }

    pub fn open_with_meta(
        data_dir: impl Into<PathBuf>,
        meta_path: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, AppError> {
        let data_dir = data_dir.into();
        let manifest = load_manifest(&data_dir.join("manifest.json"))?;
        let dimensions = queryable_dimensions(&manifest)?;
        let meta_path = meta_path.into();
        require_file(&meta_path)?;

        let metadata = MetadataReader::new(meta_path);
        let action_schemas = metadata.load_action_schemas()?;
        let schema_ids: HashSet<u32> = action_schemas.keys().copied().collect();
        metadata.validate_dimension_schema_refs(&schema_ids)?;

        for dimension in &dimensions {
            let idx_path = data_dir.join(&dimension.idx_file);
            let bin_path = data_dir.join(&dimension.bin_file);
            require_file(&idx_path)?;
            require_file(&bin_path)?;
            let reader = DimensionReader::open(&idx_path, &bin_path)?;
            for action_schema_id in reader.unique_action_schema_ids() {
                if !schema_ids.contains(&action_schema_id) {
                    return Err(AppError::action_schema_not_found(action_schema_id));
                }
            }
        }

        Ok(Self {
            action_schemas,
            metadata,
            pool: HandlePool::new(data_dir, dimensions, max_open_handles),
            verify_checksums,
        })
    }

    pub fn query(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, AppError> {
        let parsed = parse_hole_cards(hole_cards)?;
        let reader = self
            .pool
            .get_or_open(dimension)
            .map_err(|error| line_lookup_open_error(error, dimension, concrete_line_id))?;
        self.query_with_reader(&reader, dimension, concrete_line_id, parsed)
    }

    fn query_with_reader(
        &self,
        reader: &DimensionReader,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        parsed: ParsedHand,
    ) -> Result<QueryResult, AppError> {
        let fragment = reader.query(concrete_line_id, parsed.hand_id, self.verify_checksums)?;
        let Some(fragment) = fragment else {
            if reader.contains_concrete_line(concrete_line_id) {
                return Err(AppError::hand_outside_action_line(
                    &parsed.input,
                    concrete_line_id,
                    &dimension.strategy,
                    dimension.player_count,
                    dimension.depth_bb,
                ));
            }
            return Err(AppError::concrete_line_not_found(
                concrete_line_id,
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            ));
        };

        let action_schema = self
            .action_schemas
            .get(&fragment.action_schema_id)
            .ok_or_else(|| AppError::action_schema_not_found(fragment.action_schema_id))?;
        let mut actions = Vec::with_capacity(fragment.cells.len());
        for cell in fragment.cells {
            let action = action_schema.get(cell.action_id as usize).ok_or_else(|| {
                AppError::invalid_format(format!(
                    "Action id {} is outside schema {}",
                    cell.action_id, fragment.action_schema_id
                ))
            })?;
            actions.push(ActionResult {
                action_name: action.action_name.as_str().to_owned(),
                action_size: action.action_size,
                amount_bb: action.amount_bb,
                frequency: cell.frequency,
                hand_ev: cell.hand_ev,
            });
        }

        Ok(QueryResult {
            input_hole_cards: parsed.input,
            hand_code: parsed.hand_code,
            actions,
        })
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<Vec<BatchItemResult>, AppError> {
        let reader = self.pool.get_or_open(dimension)?;
        Ok(requests
            .iter()
            .map(
                |(concrete_line_id, hole_cards)| match parse_hole_cards(hole_cards) {
                    Ok(parsed) => {
                        let hand_code = parsed.hand_code.clone();
                        match self.query_with_reader(&reader, dimension, *concrete_line_id, parsed)
                        {
                            Ok(result) => BatchItemResult {
                                concrete_line_id: *concrete_line_id,
                                input_hole_cards: hole_cards.clone(),
                                hand_code: Some(result.hand_code),
                                strategy: Some(BatchStrategyResult {
                                    actions: result.actions,
                                }),
                                error: None,
                            },
                            Err(error) => BatchItemResult {
                                concrete_line_id: *concrete_line_id,
                                input_hole_cards: hole_cards.clone(),
                                hand_code: Some(hand_code),
                                strategy: None,
                                error: Some(ErrorInfo {
                                    code: error.public_code(),
                                    message: error.message().to_owned(),
                                }),
                            },
                        }
                    }
                    Err(error) => {
                        let error = AppError::from(error);
                        BatchItemResult {
                            concrete_line_id: *concrete_line_id,
                            input_hole_cards: hole_cards.clone(),
                            hand_code: None,
                            strategy: None,
                            error: Some(ErrorInfo {
                                code: error.public_code(),
                                message: error.message().to_owned(),
                            }),
                        }
                    }
                },
            )
            .collect())
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<usize, AppError> {
        let reader = self.pool.get_or_open(dimension)?;
        let expected: HashSet<u32> = self
            .metadata
            .dimension_action_schema_ids(
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            )?
            .into_iter()
            .collect();
        let actual: HashSet<u32> = reader.unique_action_schema_ids().into_iter().collect();
        if expected != actual {
            return Err(AppError::invalid_format(format!(
                "dimension_action_schemas mismatch for {}:{}max:{}BB",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            )));
        }
        Ok(self.pool.open_count())
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        abstract_line: &str,
    ) -> Result<Vec<ConcreteLineRow>, AppError> {
        self.metadata.get_concrete_lines(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            abstract_line,
        )
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, AppError> {
        let lines = self.metadata.get_drill_scenario_lines(
            strategy,
            drill_name,
            player_count,
            drill_depth,
        )?;
        if lines.is_empty() {
            return Err(AppError::drill_scenario_not_found(
                strategy,
                drill_name,
                player_count,
                drill_depth,
            ));
        }
        Ok(lines)
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: Option<Vec<ActionFilter>>,
        frequency: Option<f64>,
    ) -> Result<HandsByActionsResult, AppError> {
        let reader = self
            .pool
            .get_or_open(dimension)
            .map_err(|error| line_lookup_open_error(error, dimension, concrete_line_id))?;
        let result = reader
            .query_all(concrete_line_id, self.verify_checksums)
            .map_err(AppError::from)?;

        let Some(result) = result else {
            return Err(AppError::concrete_line_not_found(
                concrete_line_id,
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            ));
        };

        let action_schema = self
            .action_schemas
            .get(&result.action_schema_id)
            .ok_or_else(|| AppError::action_schema_not_found(result.action_schema_id))?;
        let filters = action_filters.unwrap_or_default();
        let frequency_filter = FrequencyFilter::from_request(frequency);
        let actions_text = format_action_filters(&filters);
        let required_action_groups = resolve_action_filter_groups(
            action_schema,
            &filters,
            &actions_text,
            &frequency_filter,
            dimension,
            concrete_line_id,
        )?;

        // Map each hand to actions that survive the action and frequency filters.
        let mut hand_action_matches: HashMap<u8, HashSet<u32>> = HashMap::new();
        for cell in &result.pack.cells {
            if !cell.exists || !frequency_filter.matches(cell.frequency) {
                continue;
            }
            if required_action_groups.is_empty()
                || required_action_groups
                    .iter()
                    .any(|group| group.contains(&cell.action_id))
            {
                hand_action_matches
                    .entry(cell.hand_id)
                    .or_default()
                    .insert(cell.action_id);
            }
        }

        let mut hands = Vec::new();
        for hand_id in result.pack.hand_ids {
            if let Some(matched_ids) = hand_action_matches.get(&hand_id) {
                if required_action_groups.is_empty()
                    || required_action_groups.iter().all(|group| {
                        matched_ids
                            .iter()
                            .any(|action_id| group.contains(action_id))
                    })
                {
                    hands.push(hand_code_from_id(hand_id));
                }
            }
        }

        if hands.is_empty() {
            return Err(AppError::no_hands_found(
                &actions_text,
                &frequency_filter.description(),
                concrete_line_id,
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            ));
        }

        Ok(HandsByActionsResult { hands })
    }

    pub fn schema_count(&self) -> usize {
        self.action_schemas.len()
    }

    pub fn open_handle_count(&self) -> usize {
        self.pool.open_count()
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        self.pool.known_dimensions()
    }
}

fn require_file(path: &Path) -> Result<(), AppError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(AppError::bin_file_not_found(format!(
            "Required data file not found: {}",
            path.display()
        )))
    }
}

struct FrequencyFilter {
    threshold: f64,
    include_equal: bool,
}

impl FrequencyFilter {
    fn from_request(frequency: Option<f64>) -> Self {
        match frequency {
            Some(threshold) => Self {
                threshold,
                include_equal: true,
            },
            None => Self {
                threshold: 0.0,
                include_equal: false,
            },
        }
    }

    fn matches(&self, value: f64) -> bool {
        if self.include_equal {
            value >= self.threshold
        } else {
            value > self.threshold
        }
    }

    fn description(&self) -> String {
        if self.include_equal {
            format!(">={}", self.threshold)
        } else {
            ">0".to_owned()
        }
    }
}

fn resolve_action_filter_groups(
    action_schema: &[ActionDef],
    filters: &[ActionFilter],
    actions_text: &str,
    frequency_filter: &FrequencyFilter,
    dimension: &DimensionRef,
    concrete_line_id: u32,
) -> Result<Vec<HashSet<u32>>, AppError> {
    let mut groups = Vec::with_capacity(filters.len());
    for filter in filters {
        let ids: HashSet<u32> = action_schema
            .iter()
            .filter(|action| action_matches_filter(action, filter))
            .map(|action| action.action_id)
            .collect();
        if ids.is_empty() {
            return Err(AppError::no_hands_found(
                actions_text,
                &frequency_filter.description(),
                concrete_line_id,
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            ));
        }
        groups.push(ids);
    }
    Ok(groups)
}

fn line_lookup_open_error(
    error: AppError,
    dimension: &DimensionRef,
    concrete_line_id: u32,
) -> AppError {
    if error.public_code() == 404 {
        AppError::concrete_line_not_found(
            concrete_line_id,
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        )
    } else {
        error
    }
}

fn action_matches_filter(action: &ActionDef, filter: &ActionFilter) -> bool {
    action.action_name == filter.action_name
        && match filter.amount_bb {
            Some(amount_bb) => (action.amount_bb - amount_bb).abs() <= f32::EPSILON,
            None => true,
        }
}

fn format_action_filters(filters: &[ActionFilter]) -> String {
    if filters.is_empty() {
        "[]".to_owned()
    } else {
        filters
            .iter()
            .map(|filter| filter.raw.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}
