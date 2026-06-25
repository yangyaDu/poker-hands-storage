use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use range_store_core::DimensionReader;
use serde::Serialize;

use crate::action_schema::ActionDef;
use crate::error::AppError;
use crate::hand_dict::parse_hole_cards;
use crate::manifest::{load_manifest, queryable_dimensions};
use crate::meta_db::{ConcreteLineRow, MetadataReader};
use crate::naming::DimensionRef;
use crate::pool::HandlePool;

pub struct QueryService {
    action_schemas: HashMap<u32, Vec<ActionDef>>,
    metadata: MetadataReader,
    pool: HandlePool,
    verify_checksums: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionResult {
    pub action_name: String,
    pub action_size: f32,
    pub amount_bb: f32,
    pub frequency: f64,
    pub hand_ev: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QueryResult {
    pub input_hole_cards: String,
    pub hand_code: String,
    pub exists: bool,
    pub actions: Vec<ActionResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchItemResult {
    pub concrete_line_id: u32,
    pub input_hole_cards: String,
    pub hand_code: Option<String>,
    pub strategy: Option<QueryResult>,
    pub error: Option<ErrorInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

impl QueryService {
    pub fn open(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, AppError> {
        let data_dir = data_dir.into();
        let manifest = load_manifest(&data_dir.join("manifest.json"))?;
        let dimensions = queryable_dimensions(&manifest)?;
        let meta_path = data_dir.join("meta.db");
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
        let reader = self.pool.get_or_open(dimension)?;
        let fragment = reader.query(concrete_line_id, parsed.hand_id, self.verify_checksums)?;
        let Some(fragment) = fragment else {
            return Ok(QueryResult {
                input_hole_cards: parsed.input,
                hand_code: parsed.hand_code,
                exists: false,
                actions: Vec::new(),
            });
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
            exists: true,
            actions,
        })
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Vec<BatchItemResult> {
        requests
            .iter()
            .map(|(concrete_line_id, hole_cards)| {
                match self.query(dimension, *concrete_line_id, hole_cards) {
                    Ok(result) => BatchItemResult {
                        concrete_line_id: *concrete_line_id,
                        input_hole_cards: hole_cards.clone(),
                        hand_code: Some(result.hand_code.clone()),
                        strategy: Some(result),
                        error: None,
                    },
                    Err(error) => BatchItemResult {
                        concrete_line_id: *concrete_line_id,
                        input_hole_cards: hole_cards.clone(),
                        hand_code: parse_hole_cards(hole_cards)
                            .ok()
                            .map(|parsed| parsed.hand_code),
                        strategy: None,
                        error: Some(ErrorInfo {
                            code: error.code().to_owned(),
                            message: error.to_string(),
                        }),
                    },
                }
            })
            .collect()
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
        self.metadata
            .get_drill_scenario_lines(strategy, drill_name, player_count, drill_depth)
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
