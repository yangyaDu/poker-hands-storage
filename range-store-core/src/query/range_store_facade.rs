use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::dimension::DimensionRef;
use crate::metadata::{ConcreteLineFilter, ConcreteLineRow, MetadataError, MetadataReader};

use super::hands_by_actions::{
    format_action_filters, parse_action_filters, ActionFilter, ActionFilterParseError,
};
use super::store_query_service::{
    BatchItemResult, DetailedBatchItemResult, QueryResult, StoreQueryError, StoreQueryService,
};

pub struct RangeStoreFacade {
    metadata: MetadataReader,
    metadata_cache: MetadataCache,
    query_service: StoreQueryService,
}

#[derive(Debug, Default)]
struct MetadataCache {
    concrete_line_ids: Mutex<HashMap<ConcreteLineCacheKey, u32>>,
    drill_scenario_lines: Mutex<HashMap<DrillScenarioCacheKey, Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConcreteLineCacheKey {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DrillScenarioCacheKey {
    strategy: String,
    drill_name: String,
    player_count: u32,
    drill_depth: u32,
}

#[derive(Debug)]
pub enum RangeStoreError {
    Metadata(MetadataError),
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
        let metadata = MetadataReader::new(meta_path.clone());
        let query_service = StoreQueryService::open_with_meta(
            data_dir,
            meta_path,
            max_open_handles,
            verify_checksums,
        )?;
        Ok(Self {
            metadata,
            metadata_cache: MetadataCache::default(),
            query_service,
        })
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, RangeStoreError> {
        Ok(self.metadata.get_concrete_lines(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            filter,
        )?)
    }

    pub fn get_concrete_line_id(
        &self,
        dimension: &DimensionRef,
        concrete_line: &str,
    ) -> Result<u32, RangeStoreError> {
        let cache_key = ConcreteLineCacheKey {
            strategy: dimension.strategy.clone(),
            player_count: dimension.player_count,
            depth_bb: dimension.depth_bb,
            concrete_line: concrete_line.to_owned(),
        };
        if let Ok(cache) = self.metadata_cache.concrete_line_ids.lock() {
            if let Some(id) = cache.get(&cache_key) {
                return Ok(*id);
            }
        }
        let rows =
            self.get_concrete_lines(dimension, ConcreteLineFilter::Concrete(concrete_line))?;
        let concrete_line_id = rows[0].concrete_line_id;
        if let Ok(mut cache) = self.metadata_cache.concrete_line_ids.lock() {
            cache.insert(cache_key, concrete_line_id);
        }
        Ok(concrete_line_id)
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, RangeStoreError> {
        let cache_key = DrillScenarioCacheKey {
            strategy: strategy.to_owned(),
            drill_name: drill_name.to_owned(),
            player_count,
            drill_depth,
        };
        if let Ok(cache) = self.metadata_cache.drill_scenario_lines.lock() {
            if let Some(lines) = cache.get(&cache_key) {
                return Ok(lines.clone());
            }
        }
        let lines = self.metadata.get_drill_scenario_lines(
            strategy,
            drill_name,
            player_count,
            drill_depth,
        )?;
        if let Ok(mut cache) = self.metadata_cache.drill_scenario_lines.lock() {
            cache.insert(cache_key, lines.clone());
        }
        Ok(lines)
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
    ) -> Result<Vec<BatchItemResult>, RangeStoreError> {
        Ok(self.query_service.query_batch(dimension, requests)?)
    }

    pub fn query_batch_detailed(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<Vec<DetailedBatchItemResult>, RangeStoreError> {
        Ok(self
            .query_service
            .query_batch_detailed(dimension, requests)?)
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
            Self::Query(error) => match error {
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
                StoreQueryError::HandParse(_) => "UNKNOWN_HAND",
                StoreQueryError::Io(_) => "INVALID_FORMAT",
                StoreQueryError::NotFound(_) => "CONCRETE_LINE_NOT_FOUND",
                StoreQueryError::Internal(_) => "INTERNAL",
            },
            Self::NoHandsFound { .. } => "HANDS_NOT_FOUND",
        }
    }

    pub fn public_code(&self) -> i32 {
        match self.code() {
            "UNKNOWN_HAND" | "INVALID_ARGUMENT" => 1000,
            "DIMENSION_NOT_FOUND"
            | "DATA_FILE_NOT_FOUND"
            | "CONCRETE_LINE_NOT_FOUND"
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

impl From<MetadataError> for RangeStoreError {
    fn from(error: MetadataError) -> Self {
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
