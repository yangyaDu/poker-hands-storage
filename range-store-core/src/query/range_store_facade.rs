use std::path::PathBuf;
use std::sync::Arc;

use crate::dimension::DimensionRef;
use crate::metadata::{CachedMetadataReader, ConcreteLineFilter, ConcreteLineRow};
use crate::query::hands_by_actions::{
    format_action_filters, parse_action_filters, ActionFilter, ActionFilterParseError,
};
use crate::query::store_query_service::{
    QueryBatchResult, QueryResult, StoreQueryError, StoreQueryService,
};

pub struct RangeStoreFacade {
    cached_metadata: Arc<CachedMetadataReader>,
    query_service: StoreQueryService,
}

#[derive(Debug)]
pub enum RangeStoreError {
    Metadata(crate::metadata::MetadataError),
    Query(StoreQueryError),
    NoHandsFound {
        actions: String,
        frequency: String,
        concrete_line_id: u32,
        strategy: String,
        player_count: u32,
        depth_bb: u32,
    },
}

impl RangeStoreFacade {
    pub fn open(
        data_dir: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, RangeStoreError> {
        let data_dir = data_dir.into();
        let meta_path = data_dir.join("meta.db");
        Self::open_with_meta(data_dir, meta_path, max_open_handles, verify_checksums)
    }

    pub fn open_with_meta(
        data_dir: impl Into<PathBuf>,
        meta_path: impl Into<PathBuf>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, RangeStoreError> {
        let data_dir = data_dir.into();
        let meta_path = meta_path.into();
        let cached_metadata = CachedMetadataReader::load(&data_dir, &meta_path)?;
        let query_service = StoreQueryService::open_with_meta(
            data_dir,
            meta_path,
            max_open_handles,
            verify_checksums,
        )?;
        Ok(Self {
            cached_metadata: Arc::new(cached_metadata),
            query_service,
        })
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, RangeStoreError> {
        let (abstract_line, concrete_line) = match filter {
            ConcreteLineFilter::Abstract(abstract_line) => (Some(abstract_line), None),
            ConcreteLineFilter::Concrete(concrete_line) => (None, Some(concrete_line)),
            ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            } => (Some(abstract_line), Some(concrete_line)),
        };
        Ok(self.cached_metadata.get_concrete_lines(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            abstract_line,
            concrete_line,
        )?)
    }

    /// Fast path: look up drill scenario lines from the in-memory cache.
    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, RangeStoreError> {
        Ok(self.cached_metadata.get_drill_scenario_lines(
            strategy,
            drill_name,
            player_count,
            drill_depth,
        )?)
    }

    pub fn query_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, RangeStoreError> {
        Ok(self
            .query_service
            .query(dimension, concrete_line_id, hole_cards)?)
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<QueryBatchResult, RangeStoreError> {
        Ok(self.query_service.query_batch(dimension, requests)?)
    }

    pub fn hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, RangeStoreError> {
        let hands = self.query_service.query_hands_by_actions(
            dimension,
            concrete_line_id,
            action_filters,
            frequency,
        )?;
        if hands.is_empty() {
            return Err(RangeStoreError::NoHandsFound {
                actions: format_action_filters(action_filters),
                frequency: format_frequency(frequency),
                concrete_line_id,
                strategy: dimension.strategy.clone(),
                player_count: dimension.player_count,
                depth_bb: dimension.depth_bb,
            });
        }
        Ok(hands)
    }

    pub fn hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, RangeStoreError> {
        let action_filters = parse_action_filters(action_names.to_vec())?;
        self.hands_by_actions(dimension, concrete_line_id, &action_filters, frequency)
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<usize, RangeStoreError> {
        Ok(self.query_service.prewarm(dimension)?)
    }

    pub fn schema_count(&self) -> usize {
        self.query_service.schema_count()
    }

    pub fn open_handle_count(&self) -> usize {
        self.query_service.open_handle_count()
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        self.query_service.known_dimensions()
    }
}

impl RangeStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Metadata(error) => error.code(),
            Self::Query(error) => store_query_error_code(error),
            Self::NoHandsFound { .. } => "HANDS_NOT_FOUND",
        }
    }

    pub fn public_code(&self) -> i32 {
        match self.code() {
            "INVALID_ARGUMENT" => 1000,
            "DIMENSION_NOT_FOUND"
            | "DATA_FILE_NOT_FOUND"
            | "CONCRETE_LINE_NOT_FOUND"
            | "HAND_STRATEGY_NOT_FOUND"
            | "HANDS_NOT_FOUND"
            | "ACTION_SCHEMA_NOT_FOUND"
            | "ABSTRACT_LINE_NOT_FOUND"
            | "DRILL_SCENARIO_NOT_FOUND" => 404,
            _ => 500,
        }
    }
}

impl std::fmt::Display for RangeStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Metadata(error) => write!(f, "{error}"),
            Self::Query(error) => write!(f, "{error}"),
            Self::NoHandsFound {
                actions,
                frequency,
                concrete_line_id,
                strategy,
                player_count,
                depth_bb,
            } => write!(
                f,
                "No hands found for actions={actions} at frequency{frequency}, concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
        }
    }
}

impl std::error::Error for RangeStoreError {}

impl From<crate::metadata::MetadataError> for RangeStoreError {
    fn from(error: crate::metadata::MetadataError) -> Self {
        Self::Metadata(error)
    }
}

impl From<StoreQueryError> for RangeStoreError {
    fn from(error: StoreQueryError) -> Self {
        Self::Query(error)
    }
}

impl From<ActionFilterParseError> for RangeStoreError {
    fn from(error: ActionFilterParseError) -> Self {
        Self::Query(StoreQueryError::ActionFilter(error))
    }
}

fn format_frequency(frequency: Option<f64>) -> String {
    match frequency {
        Some(0.0) => ">0".to_owned(),
        Some(value) => format!(">{value}"),
        None => ">0.005".to_owned(),
    }
}

fn store_query_error_code(error: &StoreQueryError) -> &'static str {
    match error {
        StoreQueryError::Manifest(_) => "INVALID_FORMAT",
        StoreQueryError::ActionSchema(_) => "INVALID_FORMAT",
        StoreQueryError::ActionFilter(_) => "INVALID_ARGUMENT",
        StoreQueryError::HandlePool(pool_error) => {
            if pool_error.to_string().starts_with("Dimension not found:") {
                "DIMENSION_NOT_FOUND"
            } else {
                "DATA_FILE_NOT_FOUND"
            }
        }
        StoreQueryError::InvalidArgument(_) => "INVALID_ARGUMENT",
        StoreQueryError::ActionSchemaNotFound(_) => "ACTION_SCHEMA_NOT_FOUND",
        StoreQueryError::Io(_) => "INVALID_FORMAT",
        StoreQueryError::ConcreteLineNotFound { .. } => "CONCRETE_LINE_NOT_FOUND",
        StoreQueryError::HandStrategyNotFound { .. } => "HAND_STRATEGY_NOT_FOUND",
        StoreQueryError::BatchItem { source, .. } => store_query_error_code(source),
        StoreQueryError::Internal(_) => "INTERNAL",
    }
}
