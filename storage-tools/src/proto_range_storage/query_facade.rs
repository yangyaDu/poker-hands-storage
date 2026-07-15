use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use range_store_core::dimension::{
    dimension_key, get_drill_scenario_table_name, quote_identifier, DimensionRef,
};
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::query::{ActionFilter, QueryBatchResult, QueryResult};
use range_store_core::sqlite::{Connection, Value};

use crate::errors::ToolError;

use super::format::METADATA_FILE_NAME;
use super::line_matrix_store::{
    read_compact_archive_dimension, CompactArchiveOpenOptions, MatrixCacheStats,
    DEFAULT_MATRIX_CACHE_CAPACITY,
};
use super::query_service::{ProfiledHandStrategyResult, ProtoRangeQueryService};

pub struct ProtoRangeStoreFacade {
    archive_dirs: BTreeMap<String, PathBuf>,
    options: ProtoRangeStoreFacadeOptions,
    handles: Mutex<HandlePool>,
}

#[derive(Debug, Clone)]
pub struct ProtoRangeStoreFacadeOptions {
    pub max_open_handles: usize,
    pub matrix_cache_capacity: usize,
    pub matrix_cache_byte_budget: Option<usize>,
    pub verify_checksums: bool,
}

impl Default for ProtoRangeStoreFacadeOptions {
    fn default() -> Self {
        Self {
            max_open_handles: 16,
            matrix_cache_capacity: DEFAULT_MATRIX_CACHE_CAPACITY,
            matrix_cache_byte_budget: None,
            verify_checksums: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HandlePoolStats {
    pub hits: u64,
    pub opens: u64,
    pub evictions: u64,
}

#[derive(Debug, Clone)]
pub struct FacadeProfiledHandStrategyResult {
    pub profiled: ProfiledHandStrategyResult,
    pub facade_total_ms: f64,
}

impl ProtoRangeStoreFacade {
    pub fn open(
        root_dir: impl AsRef<Path>,
        max_open_handles: usize,
        verify_checksums: bool,
    ) -> Result<Self, ToolError> {
        Self::open_with_options(
            root_dir,
            ProtoRangeStoreFacadeOptions {
                max_open_handles,
                verify_checksums,
                ..ProtoRangeStoreFacadeOptions::default()
            },
        )
    }

    pub fn open_with_options(
        root_dir: impl AsRef<Path>,
        options: ProtoRangeStoreFacadeOptions,
    ) -> Result<Self, ToolError> {
        let root_dir = root_dir.as_ref();
        if !root_dir.is_dir() {
            return Err(ToolError::invalid_argument(format!(
                "Proto range storage directory does not exist: {}",
                root_dir.display()
            )));
        }

        let mut archive_dirs = BTreeMap::new();
        for entry in fs::read_dir(root_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() || !path.join("manifest.json").is_file() {
                continue;
            }
            let stored_dimension = read_compact_archive_dimension(&path)?;
            let key = dimension_key(&DimensionRef::new(
                stored_dimension.strategy,
                stored_dimension.player_count,
                stored_dimension.depth_bb,
            ));
            if archive_dirs.insert(key.clone(), path).is_some() {
                return Err(ToolError::invalid_format(format!(
                    "Duplicate Proto archive dimension discovered: {key}"
                )));
            }
        }

        Ok(Self {
            archive_dirs,
            handles: Mutex::new(HandlePool::new(options.max_open_handles)),
            options,
        })
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        self.archive_dirs.keys().cloned().collect()
    }

    pub fn open_handle_count(&self) -> usize {
        self.handles
            .lock()
            .expect("Proto handle pool lock poisoned")
            .len()
    }

    pub fn handle_pool_stats(&self) -> HandlePoolStats {
        self.handles
            .lock()
            .expect("Proto handle pool lock poisoned")
            .stats()
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<(), ToolError> {
        self.with_service(dimension, |_| Ok(()))
    }

    pub fn matrix_count(&self, dimension: &DimensionRef) -> Result<u64, ToolError> {
        self.with_service(dimension, |service| service.matrix_count(dimension))
    }

    pub fn matrix_cache_stats(
        &self,
        dimension: &DimensionRef,
    ) -> Result<MatrixCacheStats, ToolError> {
        self.with_service(dimension, |service| Ok(service.matrix_cache_stats()))
    }

    pub fn query_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hand_strategy(dimension, concrete_line_id, hole_cards)
        })
    }

    pub fn profile_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        hole_cards: &str,
    ) -> Result<FacadeProfiledHandStrategyResult, ToolError> {
        let started = Instant::now();
        let profiled = self.with_service(dimension, |service| {
            service.profile_hand_strategy(dimension, concrete_line_id, hole_cards)
        })?;
        Ok(FacadeProfiledHandStrategyResult {
            profiled,
            facade_total_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    pub fn query_batch(
        &self,
        dimension: &DimensionRef,
        requests: &[(u32, String)],
    ) -> Result<QueryBatchResult, ToolError> {
        self.with_service(dimension, |service| {
            service.query_batch(dimension, requests)
        })
    }

    pub fn query_hands_by_actions(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hands_by_actions(dimension, concrete_line_id, action_filters, frequency)
        })
    }

    pub fn query_hands_by_action_names(
        &self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
        action_names: &[String],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hands_by_action_names(
                dimension,
                concrete_line_id,
                action_names,
                frequency,
            )
        })
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, ToolError> {
        self.with_metadata(dimension, |metadata| metadata.get_concrete_lines(filter))
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, ToolError> {
        let dimension = DimensionRef::new(strategy, player_count, drill_depth);
        self.with_metadata(&dimension, |metadata| {
            metadata.get_drill_scenario_lines(strategy, drill_name, player_count, drill_depth)
        })
    }

    fn with_service<T>(
        &self,
        dimension: &DimensionRef,
        query: impl FnOnce(&ProtoRangeQueryService) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let key = dimension_key(dimension);
        let archive_dir = self.archive_dirs.get(&key).ok_or_else(|| {
            ToolError::new(
                "DIMENSION_NOT_FOUND",
                format!("Proto range storage does not contain dimension {key}"),
            )
        })?;
        let mut handles = self
            .handles
            .lock()
            .expect("Proto handle pool lock poisoned");
        let handle = handles.get_or_open(&key, archive_dir);
        let service = handle.service(archive_dir, &self.options)?;
        query(service)
    }

    fn with_metadata<T>(
        &self,
        dimension: &DimensionRef,
        query: impl FnOnce(&mut MetadataCache) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let key = dimension_key(dimension);
        let archive_dir = self.archive_dirs.get(&key).ok_or_else(|| {
            ToolError::new(
                "DIMENSION_NOT_FOUND",
                format!("Proto range storage does not contain dimension {key}"),
            )
        })?;
        let mut handles = self
            .handles
            .lock()
            .expect("Proto handle pool lock poisoned");
        query(&mut handles.get_or_open(&key, archive_dir).metadata)
    }
}

struct HandlePool {
    capacity: usize,
    counter: u64,
    entries: HashMap<String, OpenHandle>,
    stats: HandlePoolStats,
}

struct OpenHandle {
    service: Option<ProtoRangeQueryService>,
    metadata: MetadataCache,
    last_access: u64,
}

impl OpenHandle {
    fn new(archive_dir: &Path, last_access: u64) -> Self {
        Self {
            service: None,
            metadata: MetadataCache::new(archive_dir.join(METADATA_FILE_NAME)),
            last_access,
        }
    }

    fn service(
        &mut self,
        archive_dir: &Path,
        options: &ProtoRangeStoreFacadeOptions,
    ) -> Result<&ProtoRangeQueryService, ToolError> {
        if self.service.is_none() {
            self.service = Some(ProtoRangeQueryService::open_with_options(
                archive_dir,
                CompactArchiveOpenOptions {
                    verify_checksums: options.verify_checksums,
                    cache_capacity: options.matrix_cache_capacity,
                    cache_byte_budget: options.matrix_cache_byte_budget,
                },
            )?);
        }
        Ok(self.service.as_ref().expect("opened Proto query service"))
    }
}

struct MetadataCache {
    path: PathBuf,
    connection: Option<Connection>,
    concrete_lines: HashMap<ConcreteLineCacheKey, Vec<ConcreteLineRow>>,
    drill_lines: HashMap<DrillScenarioCacheKey, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ConcreteLineCacheKey {
    Abstract(String),
    Concrete(String),
    AbstractAndConcrete {
        abstract_line: String,
        concrete_line: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DrillScenarioCacheKey {
    strategy: String,
    drill_name: String,
    player_count: u32,
    drill_depth: u32,
}

impl MetadataCache {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            connection: None,
            concrete_lines: HashMap::new(),
            drill_lines: HashMap::new(),
        }
    }

    fn get_concrete_lines(
        &mut self,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, ToolError> {
        let (key, where_clause, values) = match filter {
            ConcreteLineFilter::Abstract(abstract_line) => (
                ConcreteLineCacheKey::Abstract(abstract_line.to_owned()),
                "abstract_line = ?1",
                vec![Value::from(abstract_line)],
            ),
            ConcreteLineFilter::Concrete(concrete_line) => (
                ConcreteLineCacheKey::Concrete(concrete_line.to_owned()),
                "concrete_line = ?1",
                vec![Value::from(concrete_line)],
            ),
            ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            } => (
                ConcreteLineCacheKey::AbstractAndConcrete {
                    abstract_line: abstract_line.to_owned(),
                    concrete_line: concrete_line.to_owned(),
                },
                "abstract_line = ?1 AND concrete_line = ?2",
                vec![Value::from(abstract_line), Value::from(concrete_line)],
            ),
        };
        if let Some(lines) = self.concrete_lines.get(&key) {
            return Ok(lines.clone());
        }
        let sql = format!(
            "SELECT concrete_line_id, abstract_line, concrete_line
             FROM concrete_lines
             WHERE {where_clause}
             ORDER BY concrete_line_id"
        );
        let lines = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(&sql)?;
            statement.start(&values)?;
            let mut lines = Vec::new();
            while statement.step_row()? {
                lines.push(ConcreteLineRow {
                    concrete_line_id: statement.column_u32(0)?,
                    abstract_line: statement.column_text(1)?,
                    concrete_line: statement.column_text(2)?,
                });
            }
            lines
        };
        if lines.is_empty() {
            return Err(ToolError::new(
                "CONCRETE_LINE_NOT_FOUND",
                "No concrete lines match",
            ));
        }
        self.concrete_lines.insert(key, lines.clone());
        Ok(lines)
    }

    fn get_drill_scenario_lines(
        &mut self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, ToolError> {
        let key = DrillScenarioCacheKey {
            strategy: strategy.to_owned(),
            drill_name: drill_name.to_owned(),
            player_count,
            drill_depth,
        };
        if let Some(lines) = self.drill_lines.get(&key) {
            return Ok(lines.clone());
        }
        let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
        let sql = format!(
            "SELECT abstract_line
             FROM {table}
             WHERE drill_name = ?1 AND player_count = ?2 AND drill_depth = ?3
             ORDER BY abstract_line"
        );
        let lines = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(&sql)?;
            statement.start(&[
                Value::from(drill_name),
                Value::from(player_count),
                Value::from(drill_depth),
            ])?;
            let mut lines = Vec::new();
            while statement.step_row()? {
                lines.push(statement.column_text(0)?);
            }
            lines
        };
        if lines.is_empty() {
            return Err(ToolError::new(
                "DRILL_SCENARIO_NOT_FOUND",
                "No abstract lines found",
            ));
        }
        self.drill_lines.insert(key, lines.clone());
        Ok(lines)
    }

    fn connection(&mut self) -> Result<&Connection, ToolError> {
        if self.connection.is_none() {
            self.connection = Some(Connection::open(&self.path, true)?);
        }
        Ok(self
            .connection
            .as_ref()
            .expect("opened Proto metadata connection"))
    }
}

impl HandlePool {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            counter: 0,
            entries: HashMap::new(),
            stats: HandlePoolStats::default(),
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn stats(&self) -> HandlePoolStats {
        self.stats
    }

    fn get_or_open(&mut self, key: &str, archive_dir: &Path) -> &mut OpenHandle {
        self.counter = self.counter.wrapping_add(1);
        let access_sequence = self.counter;
        if self.entries.contains_key(key) {
            self.stats.hits = self.stats.hits.wrapping_add(1);
            self.entries
                .get_mut(key)
                .expect("existing Proto handle")
                .last_access = access_sequence;
        } else {
            self.stats.opens = self.stats.opens.wrapping_add(1);
            if self.entries.len() >= self.capacity {
                if let Some(lru_key) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, handle)| handle.last_access)
                    .map(|(key, _)| key.clone())
                {
                    self.entries.remove(&lru_key);
                    self.stats.evictions = self.stats.evictions.wrapping_add(1);
                }
            }
            self.entries.insert(
                key.to_owned(),
                OpenHandle::new(archive_dir, access_sequence),
            );
        }
        self.entries.get_mut(key).expect("inserted Proto handle")
    }
}
