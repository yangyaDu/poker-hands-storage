use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use range_store_core::DimensionReader;
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::AppError;
use crate::query::dimension_handle_pool::HandlePool;
use crate::storage::manifest::{load_manifest, queryable_dimensions};
use crate::storage::metadata::{ConcreteLineFilter, ConcreteLineRow, MetadataReader};
use range_store_core::action_schema::ActionDef;
use range_store_core::dimension::DimensionRef;
use range_store_core::hole_cards::{parse_hole_cards, ParsedHand};
use range_store_core::query::{
    format_action_filters, match_hands_by_actions, ActionFilter, FrequencyFilter,
};

pub struct QueryService {
    action_schemas: Vec<Option<Vec<ActionDef>>>,
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
        let schemas_map = metadata.load_action_schemas()?;
        let schema_ids: HashSet<u32> = schemas_map.keys().copied().collect();
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

        // Convert HashMap to Vec for O(1) index lookup
        let max_id = schemas_map.keys().copied().max().unwrap_or(0) as usize;
        let mut action_schemas = vec![None; max_id + 1];
        for (id, schema) in schemas_map {
            action_schemas[id as usize] = Some(schema);
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
            .get_action_schema(fragment.action_schema_id)
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

        // Phase 1: parse all hole cards and group by concrete_line_id
        struct ParsedItem {
            original_index: usize,
            concrete_line_id: u32,
            input_hole_cards: String,
            parsed: Result<ParsedHand, AppError>,
        }

        let items: Vec<ParsedItem> = requests
            .iter()
            .enumerate()
            .map(|(i, (line_id, hole_cards))| ParsedItem {
                original_index: i,
                concrete_line_id: *line_id,
                input_hole_cards: hole_cards.clone(),
                parsed: parse_hole_cards(hole_cards).map_err(AppError::from),
            })
            .collect();

        // Phase 2: group valid items by concrete_line_id
        let mut groups: HashMap<u32, Vec<usize>> = HashMap::new();
        for (idx, item) in items.iter().enumerate() {
            if item.parsed.is_ok() {
                groups.entry(item.concrete_line_id).or_default().push(idx);
            }
        }

        // Phase 3: batch query each group (one idx lookup + one bin read per group)
        let mut results: Vec<Option<BatchItemResult>> = vec![None; requests.len()];

        for (concrete_line_id, group_indices) in &groups {
            let hand_ids: Vec<u8> = group_indices
                .iter()
                .map(|&idx| items[idx].parsed.as_ref().unwrap().hand_id)
                .collect();

            match reader.query_many_hands(*concrete_line_id, &hand_ids, self.verify_checksums) {
                Ok(Some((action_schema_id, pack_results))) => {
                    let action_schema = self.get_action_schema(action_schema_id);
                    for (group_pos, &item_idx) in group_indices.iter().enumerate() {
                        let item = &items[item_idx];
                        let parsed = item.parsed.as_ref().unwrap();
                        let result = match &pack_results[group_pos] {
                            Some(fragment) => match action_schema {
                                Some(schema) => {
                                    let actions =
                                        self.build_action_results(schema, &fragment.cells);
                                    match actions {
                                        Ok(actions) => BatchItemResult {
                                            concrete_line_id: item.concrete_line_id,
                                            input_hole_cards: item.input_hole_cards.clone(),
                                            hand_code: Some(parsed.hand_code.clone()),
                                            strategy: Some(BatchStrategyResult { actions }),
                                            error: None,
                                        },
                                        Err(e) => BatchItemResult {
                                            concrete_line_id: item.concrete_line_id,
                                            input_hole_cards: item.input_hole_cards.clone(),
                                            hand_code: Some(parsed.hand_code.clone()),
                                            strategy: None,
                                            error: Some(ErrorInfo {
                                                code: e.public_code(),
                                                message: e.message().to_owned(),
                                            }),
                                        },
                                    }
                                }
                                None => BatchItemResult {
                                    concrete_line_id: item.concrete_line_id,
                                    input_hole_cards: item.input_hole_cards.clone(),
                                    hand_code: Some(parsed.hand_code.clone()),
                                    strategy: None,
                                    error: Some(ErrorInfo {
                                        code: 500,
                                        message: format!(
                                            "Action schema {} not found",
                                            action_schema_id
                                        ),
                                    }),
                                },
                            },
                            None => {
                                // Hand not found in pack
                                let error = if reader.contains_concrete_line(*concrete_line_id) {
                                    AppError::hand_outside_action_line(
                                        &parsed.input,
                                        *concrete_line_id,
                                        &dimension.strategy,
                                        dimension.player_count,
                                        dimension.depth_bb,
                                    )
                                } else {
                                    AppError::concrete_line_not_found(
                                        *concrete_line_id,
                                        &dimension.strategy,
                                        dimension.player_count,
                                        dimension.depth_bb,
                                    )
                                };
                                BatchItemResult {
                                    concrete_line_id: item.concrete_line_id,
                                    input_hole_cards: item.input_hole_cards.clone(),
                                    hand_code: Some(parsed.hand_code.clone()),
                                    strategy: None,
                                    error: Some(ErrorInfo {
                                        code: error.public_code(),
                                        message: error.message().to_owned(),
                                    }),
                                }
                            }
                        };
                        results[item.original_index] = Some(result);
                    }
                }
                Ok(None) => {
                    // Concrete line not found — error for all items in group
                    let error = AppError::concrete_line_not_found(
                        *concrete_line_id,
                        &dimension.strategy,
                        dimension.player_count,
                        dimension.depth_bb,
                    );
                    for &item_idx in group_indices {
                        let item = &items[item_idx];
                        let parsed = item.parsed.as_ref().unwrap();
                        results[item.original_index] = Some(BatchItemResult {
                            concrete_line_id: item.concrete_line_id,
                            input_hole_cards: item.input_hole_cards.clone(),
                            hand_code: Some(parsed.hand_code.clone()),
                            strategy: None,
                            error: Some(ErrorInfo {
                                code: error.public_code(),
                                message: error.message().to_owned(),
                            }),
                        });
                    }
                }
                Err(io_error) => {
                    let error = AppError::from(io_error);
                    for &item_idx in group_indices {
                        let item = &items[item_idx];
                        let parsed = item.parsed.as_ref().unwrap();
                        results[item.original_index] = Some(BatchItemResult {
                            concrete_line_id: item.concrete_line_id,
                            input_hole_cards: item.input_hole_cards.clone(),
                            hand_code: Some(parsed.hand_code.clone()),
                            strategy: None,
                            error: Some(ErrorInfo {
                                code: error.public_code(),
                                message: error.message().to_owned(),
                            }),
                        });
                    }
                }
            }
        }

        // Phase 4: fill in parse-error items
        for (i, item) in items.iter().enumerate() {
            if results[i].is_none() {
                let error = item.parsed.as_ref().unwrap_err();
                results[i] = Some(BatchItemResult {
                    concrete_line_id: item.concrete_line_id,
                    input_hole_cards: item.input_hole_cards.clone(),
                    hand_code: None,
                    strategy: None,
                    error: Some(ErrorInfo {
                        code: error.public_code(),
                        message: error.message().to_owned(),
                    }),
                });
            }
        }

        Ok(results.into_iter().map(|r| r.unwrap()).collect())
    }

    /// Build action results from decoded cells using action schema.
    fn build_action_results(
        &self,
        action_schema: &[ActionDef],
        cells: &[range_store_core::types::DecodedCellResult],
    ) -> Result<Vec<ActionResult>, AppError> {
        let mut actions = Vec::with_capacity(cells.len());
        for cell in cells {
            let action = action_schema.get(cell.action_id as usize).ok_or_else(|| {
                AppError::invalid_format(format!("Action id {} is outside schema", cell.action_id))
            })?;
            actions.push(ActionResult {
                action_name: action.action_name.as_str().to_owned(),
                action_size: action.action_size,
                amount_bb: action.amount_bb,
                frequency: cell.frequency,
                hand_ev: cell.hand_ev,
            });
        }
        Ok(actions)
    }

    /// O(1) action schema lookup by index.
    #[inline]
    fn get_action_schema(&self, id: u32) -> Option<&Vec<ActionDef>> {
        self.action_schemas
            .get(id as usize)
            .and_then(|opt| opt.as_ref())
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
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, AppError> {
        Ok(self.metadata.get_concrete_lines(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            filter,
        )?)
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
            .get_action_schema(result.action_schema_id)
            .ok_or_else(|| AppError::action_schema_not_found(result.action_schema_id))?;
        let filters = action_filters.unwrap_or_default();
        let frequency_filter = FrequencyFilter::from_request(frequency);
        let actions_text = format_action_filters(&filters);
        let hands = match_hands_by_actions(result.pack, action_schema, &filters, &frequency_filter);

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
        self.action_schemas.iter().filter(|s| s.is_some()).count()
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
