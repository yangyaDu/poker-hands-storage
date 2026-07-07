use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use crate::action_schema::{load_action_schema_from_connection, ActionDef, ActionSchemaLoadError};
use crate::dimension::DimensionRef;
use crate::hole_cards::{parse_hole_cards, HandDictError};
use crate::manifest::{queryable_dimensions, ManifestError};
use crate::sqlite::{Connection, SqliteError};
use crate::DimensionReader;

use super::handle_pool::{HandlePool, HandlePoolError};
use super::hands_by_actions::{
    match_hands_by_actions, parse_action_filters, ActionFilter, ActionFilterParseError,
    FrequencyFilter,
};

pub const DEFAULT_HANDS_BY_ACTIONS_FREQUENCY: f64 = 0.005;

/// A lightweight query service for Range Strata Binary stores.
///
/// This provides the core query logic without any HTTP/API dependencies.
/// The `service` crate wraps this with HTTP error handling and OpenAPI types.
#[derive(Debug)]
pub struct StoreQueryService {
    action_schemas: ActionSchemaCache,
    pool: HandlePool,
    verify_checksums: bool,
}

/// Result of a single hand query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Ordered action strategy entries.
    pub actions: Vec<ActionResult>,
}

/// Result of a strict batch hand query.
#[derive(Debug, Clone)]
pub struct QueryBatchResult {
    pub results: Vec<QueryBatchItemResult>,
}

/// A successful item in a strict batch hand query.
#[derive(Debug, Clone)]
pub struct QueryBatchItemResult {
    pub concrete_line_id: u32,
    pub hole_cards: String,
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
    ActionFilter(ActionFilterParseError),
    HandlePool(HandlePoolError),
    InvalidArgument(String),
    ActionSchemaNotFound(u32),
    Io(String),
    ConcreteLineNotFound {
        dimension: DimensionRef,
        concrete_line_id: u32,
    },
    HandStrategyNotFound {
        dimension: DimensionRef,
        concrete_line_id: u32,
        hole_cards: String,
    },
    BatchItem {
        index: usize,
        concrete_line_id: u32,
        hole_cards: String,
        dimension: DimensionRef,
        source: Box<StoreQueryError>,
    },
    Internal(String),
}

impl StoreQueryError {
    pub fn public_code(&self) -> i32 {
        match self {
            Self::InvalidArgument(_) | Self::ActionFilter(_) => 1000,
            Self::HandlePool(_)
            | Self::ConcreteLineNotFound { .. }
            | Self::HandStrategyNotFound { .. }
            | Self::ActionSchemaNotFound(_) => 404,
            Self::BatchItem { source, .. } => source.public_code(),
            Self::Manifest(_) | Self::ActionSchema(_) | Self::Io(_) | Self::Internal(_) => 500,
        }
    }
}

impl std::fmt::Display for StoreQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manifest(e) => write!(f, "Manifest error: {e}"),
            Self::ActionSchema(e) => write!(f, "Action schema error: {e}"),
            Self::ActionFilter(e) => write!(f, "{e}"),
            Self::HandlePool(e) => write!(f, "{e}"),
            Self::InvalidArgument(message) => write!(f, "{message}"),
            Self::ActionSchemaNotFound(action_schema_id) => {
                write!(f, "Action schema {action_schema_id} not found")
            }
            Self::Io(msg) => write!(f, "IO error: {msg}"),
            Self::ConcreteLineNotFound {
                dimension,
                concrete_line_id,
            } => write!(
                f,
                "Concrete line not found: concrete_line_id={concrete_line_id}, dimension={}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ),
            Self::HandStrategyNotFound {
                dimension,
                concrete_line_id,
                hole_cards,
            } => write!(
                f,
                "Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ),
            Self::BatchItem {
                index,
                concrete_line_id,
                dimension,
                source,
                ..
            } => write!(
                f,
                "Batch item requests[{index}] failed: {source} from concrete_line_id={concrete_line_id}, dimension={}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ),
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

impl From<ActionFilterParseError> for StoreQueryError {
    fn from(error: ActionFilterParseError) -> Self {
        Self::ActionFilter(error)
    }
}

impl From<HandlePoolError> for StoreQueryError {
    fn from(error: HandlePoolError) -> Self {
        Self::HandlePool(error)
    }
}

impl From<HandDictError> for StoreQueryError {
    fn from(error: HandDictError) -> Self {
        Self::InvalidArgument(error.to_string())
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

        Ok(Self {
            action_schemas: ActionSchemaCache::new(meta_path)?,
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
            return Err(missing_hand_or_line_error(
                &reader,
                dimension,
                concrete_line_id,
                hole_cards,
            ));
        };

        let action_schema = self.action_schemas.get(fragment.action_schema_id)?;
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

        Ok(QueryResult { actions })
    }

    /// Query a batch of (concrete_line_id, hole_cards) pairs.
    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<QueryBatchResult, StoreQueryError> {
        let mut results = Vec::with_capacity(requests.len());
        for (index, (concrete_line_id, hole_cards)) in requests.iter().enumerate() {
            let item = self
                .query(dimension, *concrete_line_id, hole_cards)
                .map_err(|source| StoreQueryError::BatchItem {
                    index,
                    concrete_line_id: *concrete_line_id,
                    hole_cards: hole_cards.clone(),
                    dimension: dimension.clone(),
                    source: Box::new(source),
                })?;
            results.push(QueryBatchItemResult {
                concrete_line_id: *concrete_line_id,
                hole_cards: hole_cards.clone(),
                actions: item.actions,
            });
        }
        Ok(QueryBatchResult { results })
    }

    /// Query all hands in a concrete line that match the requested action filters.
    ///
    /// Empty filters mean no action restriction. Non-empty filters use OR
    /// semantics: any requested action filter can include the hand above the
    /// strict greater-than frequency threshold.
    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, StoreQueryError> {
        let reader = self.pool.get_or_open(dimension)?;
        let result = reader
            .query_all(concrete_line_id, self.verify_checksums)
            .map_err(|e| StoreQueryError::Io(e.to_string()))?;
        let Some(result) = result else {
            return Err(concrete_line_not_found_error(dimension, concrete_line_id));
        };

        let action_schema = self.action_schemas.get(result.action_schema_id)?;
        let frequency_filter = FrequencyFilter::from_request(frequency);
        Ok(match_hands_by_actions(
            result.pack,
            action_schema.as_ref(),
            action_filters,
            &frequency_filter,
        ))
    }

    /// Compatibility wrapper for callers that still pass raw action strings.
    pub fn query_hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, StoreQueryError> {
        let action_filters = parse_action_filters(action_names.to_vec())?;
        self.query_hands_by_actions(dimension, concrete_line_id, &action_filters, frequency)
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

#[derive(Debug)]
struct ActionSchemaCache {
    connection: Mutex<LockedActionSchemaConnection>,
    state: RwLock<ActionSchemaCacheState>,
}

#[derive(Debug, Default)]
struct ActionSchemaCacheState {
    schemas: HashMap<u32, Arc<Vec<ActionDef>>>,
}

struct LockedActionSchemaConnection {
    connection: Connection,
}

impl std::fmt::Debug for LockedActionSchemaConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockedActionSchemaConnection")
            .finish_non_exhaustive()
    }
}

// The SQLite handle is opened with SQLITE_OPEN_NOMUTEX and is only touched while
// this private wrapper is held behind ActionSchemaCache's Mutex.
unsafe impl Send for LockedActionSchemaConnection {}

impl ActionSchemaCache {
    fn new(meta_path: PathBuf) -> Result<Self, StoreQueryError> {
        let connection = Connection::open(&meta_path, true).map_err(action_schema_sqlite_error)?;
        Ok(Self {
            connection: Mutex::new(LockedActionSchemaConnection { connection }),
            state: RwLock::new(ActionSchemaCacheState::default()),
        })
    }

    fn get(&self, schema_id: u32) -> Result<Arc<Vec<ActionDef>>, StoreQueryError> {
        {
            let state = self.state.read().map_err(|_| {
                StoreQueryError::Internal("Action schema cache lock poisoned".to_owned())
            })?;
            if let Some(schema) = state.schemas.get(&schema_id) {
                return Ok(Arc::clone(schema));
            }
        }

        let connection = self.connection()?;
        let schema = load_action_schema_from_connection(&connection.connection, schema_id)?
            .ok_or(StoreQueryError::ActionSchemaNotFound(schema_id))?;
        drop(connection);

        let mut state = self.state.write().map_err(|_| {
            StoreQueryError::Internal("Action schema cache lock poisoned".to_owned())
        })?;
        Ok(Arc::clone(
            state
                .schemas
                .entry(schema_id)
                .or_insert_with(|| Arc::new(schema)),
        ))
    }

    fn connection(&self) -> Result<MutexGuard<'_, LockedActionSchemaConnection>, StoreQueryError> {
        self.connection
            .lock()
            .map_err(|_| StoreQueryError::Internal("Action schema cache lock poisoned".to_owned()))
    }

    fn len(&self) -> usize {
        self.state
            .read()
            .map(|state| state.schemas.len())
            .unwrap_or_default()
    }
}

fn action_schema_sqlite_error(error: SqliteError) -> StoreQueryError {
    StoreQueryError::ActionSchema(ActionSchemaLoadError::Sqlite(error))
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

fn missing_hand_or_line_error(
    reader: &DimensionReader,
    dimension: &DimensionRef,
    concrete_line_id: u32,
    hole_cards: &str,
) -> StoreQueryError {
    if reader.contains_concrete_line(concrete_line_id) {
        StoreQueryError::HandStrategyNotFound {
            dimension: dimension.clone(),
            concrete_line_id,
            hole_cards: hole_cards.to_owned(),
        }
    } else {
        concrete_line_not_found_error(dimension, concrete_line_id)
    }
}

fn concrete_line_not_found_error(
    dimension: &DimensionRef,
    concrete_line_id: u32,
) -> StoreQueryError {
    StoreQueryError::ConcreteLineNotFound {
        dimension: dimension.clone(),
        concrete_line_id,
    }
}
