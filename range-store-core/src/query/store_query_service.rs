use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::action_schema::{load_action_schemas, ActionDef, ActionSchemaLoadError};
use crate::dimension::DimensionRef;
use crate::hole_cards::{hand_code_from_id, parse_hole_cards, HandDictError};
use crate::manifest::{queryable_dimensions, ManifestError};
use crate::DimensionReader;

use super::handle_pool::{HandlePool, HandlePoolError};

pub const DEFAULT_HANDS_BY_ACTIONS_FREQUENCY: f64 = 0.005;

/// A lightweight query service for Range Strata Binary stores.
///
/// This provides the core query logic without any HTTP/API dependencies.
/// The `service` crate wraps this with HTTP error handling and OpenAPI types.
pub struct StoreQueryService {
    action_schemas: HashMap<u32, Vec<ActionDef>>,
    pool: HandlePool,
    verify_checksums: bool,
}

/// Result of a single hand query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Original hole-card input.
    pub input_hole_cards: String,
    /// Normalized 169-hand code.
    pub hand_code: String,
    /// Ordered action strategy entries.
    pub actions: Vec<ActionResult>,
}

/// A single action entry in a query result.
#[derive(Debug, Clone)]
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

/// Error type for [`StoreQueryService`] operations.
#[derive(Debug)]
pub enum StoreQueryError {
    Manifest(ManifestError),
    ActionSchema(ActionSchemaLoadError),
    HandlePool(HandlePoolError),
    HandParse(HandDictError),
    Io(String),
    NotFound(String),
    Internal(String),
}

impl std::fmt::Display for StoreQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manifest(e) => write!(f, "Manifest error: {e}"),
            Self::ActionSchema(e) => write!(f, "Action schema error: {e}"),
            Self::HandlePool(e) => write!(f, "{e}"),
            Self::HandParse(e) => write!(f, "{e}"),
            Self::Io(msg) => write!(f, "IO error: {msg}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
            Self::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl std::error::Error for StoreQueryError {}

impl From<ManifestError> for StoreQueryError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<ActionSchemaLoadError> for StoreQueryError {
    fn from(error: ActionSchemaLoadError) -> Self {
        Self::ActionSchema(error)
    }
}

impl From<HandlePoolError> for StoreQueryError {
    fn from(error: HandlePoolError) -> Self {
        Self::HandlePool(error)
    }
}

impl From<HandDictError> for StoreQueryError {
    fn from(error: HandDictError) -> Self {
        Self::HandParse(error)
    }
}

impl StoreQueryService {
    /// Open a store for querying.
    pub fn open(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, StoreQueryError> {
        let data_dir = data_dir.into();
        let meta_path = data_dir.join("meta.db");
        Self::open_with_meta(data_dir, meta_path, max_open_handles, verify_checksums)
    }

    /// Open a store with an explicit meta.db path.
    pub fn open_with_meta(
        data_dir: impl Into<PathBuf>,
        meta_path: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, StoreQueryError> {
        let data_dir = data_dir.into();
        let manifest = crate::manifest::load_manifest(&data_dir.join("manifest.json"))?;
        let dimensions = queryable_dimensions(&manifest)?;
        let meta_path = meta_path.into();
        require_file(&meta_path)?;

        let action_schemas = load_action_schemas(&meta_path)?;
        let schema_ids: HashSet<u32> = action_schemas.keys().copied().collect();

        for dimension in &dimensions {
            let idx_path = data_dir.join(&dimension.idx_file);
            let bin_path = data_dir.join(&dimension.bin_file);
            require_file(&idx_path)?;
            require_file(&bin_path)?;
            let reader = DimensionReader::open(&idx_path, &bin_path)
                .map_err(|e| StoreQueryError::Io(e.to_string()))?;
            for action_schema_id in reader.unique_action_schema_ids() {
                if !schema_ids.contains(&action_schema_id) {
                    return Err(StoreQueryError::NotFound(format!(
                        "Action schema {action_schema_id} referenced in index but not in meta.db"
                    )));
                }
            }
        }

        Ok(Self {
            action_schemas,
            pool: HandlePool::new(data_dir, dimensions, max_open_handles),
            verify_checksums,
        })
    }

    /// Query a single concrete line + hand.
    pub fn query(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, StoreQueryError> {
        let parsed = parse_hole_cards(hole_cards)?;
        let reader = self.pool.get_or_open(dimension)?;
        let fragment = reader
            .query(concrete_line_id, parsed.hand_id, self.verify_checksums)
            .map_err(|e| StoreQueryError::Io(e.to_string()))?;
        let Some(fragment) = fragment else {
            return Err(StoreQueryError::NotFound(format!(
                "concrete_line_id={concrete_line_id} or hand={hole_cards} not found"
            )));
        };

        let action_schema = self
            .action_schemas
            .get(&fragment.action_schema_id)
            .ok_or_else(|| {
                StoreQueryError::NotFound(format!(
                    "Action schema {} not found",
                    fragment.action_schema_id
                ))
            })?;
        let mut actions = Vec::with_capacity(fragment.cells.len());
        for cell in fragment.cells {
            let action = action_schema.get(cell.action_id as usize).ok_or_else(|| {
                StoreQueryError::Internal(format!(
                    "Action id {} outside schema {}",
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

    /// Query a batch of (concrete_line_id, hole_cards) pairs.
    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<Vec<BatchItemResult>, StoreQueryError> {
        let reader = self.pool.get_or_open(dimension)?;
        Ok(requests
            .iter()
            .map(|(concrete_line_id, hole_cards)| {
                match self.query_single(&reader, dimension, *concrete_line_id, hole_cards) {
                    Ok(result) => BatchItemResult {
                        concrete_line_id: *concrete_line_id,
                        input_hole_cards: hole_cards.clone(),
                        actions: Some(result.actions),
                        error: None,
                    },
                    Err(error) => BatchItemResult {
                        concrete_line_id: *concrete_line_id,
                        input_hole_cards: hole_cards.clone(),
                        actions: None,
                        error: Some(error.to_string()),
                    },
                }
            })
            .collect())
    }

    /// Query all hands in a concrete line that match any requested action name.
    ///
    /// `action_names` uses OR semantics. An empty list means no action-name
    /// restriction. `frequency` is always strict greater-than, matching the API
    /// contract for `/range/hands-by-actions`.
    pub fn query_hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, StoreQueryError> {
        let reader = self.pool.get_or_open(dimension)?;
        let result = reader
            .query_all(concrete_line_id, self.verify_checksums)
            .map_err(|e| StoreQueryError::Io(e.to_string()))?;
        let Some(result) = result else {
            return Err(StoreQueryError::NotFound(format!(
                "concrete_line_id={concrete_line_id} not found"
            )));
        };

        let action_schema = self
            .action_schemas
            .get(&result.action_schema_id)
            .ok_or_else(|| {
                StoreQueryError::NotFound(format!(
                    "Action schema {} not found",
                    result.action_schema_id
                ))
            })?;
        let action_name_filter = action_names
            .iter()
            .map(|name| name.as_str())
            .collect::<HashSet<_>>();
        let threshold = frequency.unwrap_or(DEFAULT_HANDS_BY_ACTIONS_FREQUENCY);
        let action_filter_mask =
            action_name_bitmask(action_schema, &action_name_filter).unwrap_or_default();

        if !action_name_filter.is_empty() && action_filter_mask == 0 {
            return Ok(Vec::new());
        }

        let mut hand_masks = [0u32; 169];
        for cell in &result.pack.cells {
            if !cell.exists || cell.frequency <= threshold || cell.action_id >= 32 {
                continue;
            }
            let action_bit = 1u32 << cell.action_id;
            if action_filter_mask == 0 || action_bit & action_filter_mask != 0 {
                hand_masks[cell.hand_id as usize] |= action_bit;
            }
        }

        Ok(result
            .pack
            .hand_ids
            .into_iter()
            .filter(|hand_id| hand_masks[*hand_id as usize] != 0)
            .map(hand_code_from_id)
            .collect())
    }

    fn query_single(
        &self,
        reader: &DimensionReader,
        _dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, StoreQueryError> {
        let parsed = parse_hole_cards(hole_cards)?;
        let fragment = reader
            .query(concrete_line_id, parsed.hand_id, self.verify_checksums)
            .map_err(|e| StoreQueryError::Io(e.to_string()))?;
        let Some(fragment) = fragment else {
            return Err(StoreQueryError::NotFound(format!(
                "concrete_line_id={concrete_line_id} hand={hole_cards}"
            )));
        };

        let action_schema = self
            .action_schemas
            .get(&fragment.action_schema_id)
            .ok_or_else(|| {
                StoreQueryError::NotFound(format!(
                    "Action schema {} not found",
                    fragment.action_schema_id
                ))
            })?;
        let mut actions = Vec::with_capacity(fragment.cells.len());
        for cell in fragment.cells {
            if let Some(action) = action_schema.get(cell.action_id as usize) {
                actions.push(ActionResult {
                    action_name: action.action_name.as_str().to_owned(),
                    action_size: action.action_size,
                    amount_bb: action.amount_bb,
                    frequency: cell.frequency,
                    hand_ev: cell.hand_ev,
                });
            }
        }

        Ok(QueryResult {
            input_hole_cards: parsed.input,
            hand_code: parsed.hand_code,
            actions,
        })
    }

    /// Prewarm a dimension by opening its files.
    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<usize, StoreQueryError> {
        let _reader = self.pool.get_or_open(dimension)?;
        Ok(self.pool.open_count())
    }

    /// Number of action schemas loaded.
    pub fn schema_count(&self) -> usize {
        self.action_schemas.len()
    }

    /// Number of currently open dimension handles.
    pub fn open_handle_count(&self) -> usize {
        self.pool.open_count()
    }

    /// List known dimension keys.
    pub fn known_dimensions(&self) -> Vec<String> {
        self.pool.known_dimensions()
    }
}

fn action_name_bitmask(action_schema: &[ActionDef], action_names: &HashSet<&str>) -> Option<u32> {
    if action_names.is_empty() {
        return Some(0);
    }
    let mut mask = 0u32;
    for action in action_schema {
        if action.action_id < 32 && action_names.contains(action.action_name.as_str()) {
            mask |= 1u32 << action.action_id;
        }
    }
    Some(mask)
}

/// Result of a single batch item.
#[derive(Debug, Clone)]
pub struct BatchItemResult {
    pub concrete_line_id: u32,
    pub input_hole_cards: String,
    pub actions: Option<Vec<ActionResult>>,
    pub error: Option<String>,
}

fn require_file(path: &Path) -> Result<(), StoreQueryError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(StoreQueryError::Io(format!(
            "Required file not found: {}",
            path.display()
        )))
    }
}
