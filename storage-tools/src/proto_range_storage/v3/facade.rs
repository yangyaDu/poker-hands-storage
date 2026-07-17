use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use range_store_core::dimension::{dimension_key, DimensionRef};
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::query::{ActionFilter, QueryBatchResult, QueryResult};
use serde::Serialize;

use crate::errors::ToolError;

use super::archive::V3ArchiveOpenOptions;
use super::cache::ByteCacheStats;
use super::manifest::{read_manifest, MANIFEST_FILE_NAME};
use super::query_service::V3QueryService;

#[derive(Debug, Clone)]
pub struct V3FacadeOptions {
    pub max_open_handles: usize,
    pub verify_file_checksums: bool,
    /// Facade-wide metadata cache budget, dynamically divided among open dimension handles.
    pub metadata_cache_byte_budget: usize,
    /// Facade-wide decoded-strategy cache budget, dynamically divided among open dimension handles.
    pub strategy_cache_byte_budget: usize,
}

impl Default for V3FacadeOptions {
    fn default() -> Self {
        Self {
            max_open_handles: 16,
            verify_file_checksums: false,
            metadata_cache_byte_budget: super::metadata_store::DEFAULT_METADATA_CACHE_BYTE_BUDGET,
            strategy_cache_byte_budget: super::strategy_store::DEFAULT_STRATEGY_CACHE_BYTE_BUDGET,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlePoolStats {
    pub hits: u64,
    pub opens: u64,
    pub evictions: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacadeCacheStats {
    pub open_handle_count: usize,
    /// Current metadata-cache sub-budget assigned to each open handle.
    pub metadata_per_handle_byte_budget: usize,
    /// Current decoded-strategy-cache sub-budget assigned to each open handle.
    pub strategy_per_handle_byte_budget: usize,
    pub metadata: ByteCacheStats,
    pub strategies: ByteCacheStats,
}

pub struct V3Facade {
    archive_dirs: HashMap<DimensionRef, PathBuf>,
    options: V3FacadeOptions,
    handles: Mutex<HandlePool>,
}

impl V3Facade {
    pub fn open(root_dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(root_dir, V3FacadeOptions::default())
    }

    pub fn open_with_options(
        root_dir: impl AsRef<Path>,
        options: V3FacadeOptions,
    ) -> Result<Self, ToolError> {
        if options.max_open_handles == 0 {
            return Err(ToolError::invalid_argument(
                "V3 facade max_open_handles must be positive",
            ));
        }
        let root_dir = root_dir.as_ref();
        if !root_dir.is_dir() {
            return Err(ToolError::invalid_argument(format!(
                "V3 storage root does not exist: {}",
                root_dir.display()
            )));
        }
        let mut archive_dirs = HashMap::new();
        for entry in fs::read_dir(root_dir)? {
            let path = entry?.path();
            if !path.is_dir() || !path.join(MANIFEST_FILE_NAME).is_file() {
                continue;
            }
            let manifest = read_manifest(&path)?;
            let dimension =
                DimensionRef::new(manifest.strategy, manifest.player_count, manifest.depth_bb);
            if archive_dirs.insert(dimension.clone(), path).is_some() {
                return Err(ToolError::new(
                    "INVALID_V3_MANIFEST",
                    format!("Duplicate V3 dimension {}", dimension_key(&dimension)),
                ));
            }
        }
        Ok(Self {
            archive_dirs,
            handles: Mutex::new(HandlePool::new(options.max_open_handles)),
            options,
        })
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        let mut dimensions = self
            .archive_dirs
            .keys()
            .map(dimension_key)
            .collect::<Vec<_>>();
        dimensions.sort();
        dimensions
    }

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<(), ToolError> {
        self.with_service(dimension, |_| Ok(()))
    }

    pub fn query_hand_strategy(
        &self,
        dimension: &DimensionRef,
        concrete_action_path_id: u32,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hand_strategy(dimension, concrete_action_path_id, hole_cards)
        })
    }

    pub fn query_hand_strategy_by_path(
        &self,
        dimension: &DimensionRef,
        concrete_action_path: &str,
        hole_cards: &str,
    ) -> Result<QueryResult, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hand_strategy_by_path(dimension, concrete_action_path, hole_cards)
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
        concrete_action_path_id: u32,
        action_filters: &[ActionFilter],
        frequency: Option<f64>,
    ) -> Result<Vec<String>, ToolError> {
        self.with_service(dimension, |service| {
            service.query_hands_by_actions(
                dimension,
                concrete_action_path_id,
                action_filters,
                frequency,
            )
        })
    }

    pub fn get_concrete_lines(
        &self,
        dimension: &DimensionRef,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, ToolError> {
        self.with_service(dimension, |service| {
            service.get_concrete_lines(dimension, filter)
        })
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Result<Vec<String>, ToolError> {
        let dimension = DimensionRef::new(strategy, player_count, depth_bb);
        self.with_service(&dimension, |service| {
            service.get_drill_scenario_lines(&dimension, drill_name)
        })
    }

    pub fn handle_pool_stats(&self) -> HandlePoolStats {
        self.handles
            .lock()
            .expect("V3 handle pool lock poisoned")
            .stats
    }

    pub fn cache_stats(&self) -> FacadeCacheStats {
        let handles = self.handles.lock().expect("V3 handle pool lock poisoned");
        let open_handle_count = handles.entries.len();
        let mut stats = FacadeCacheStats {
            open_handle_count,
            ..FacadeCacheStats::default()
        };
        for (index, handle) in handles.entries.values().enumerate() {
            let (metadata_cache_byte_budget, strategy_cache_byte_budget) =
                handle.service.cache_budgets();
            if index == 0 {
                stats.metadata_per_handle_byte_budget = metadata_cache_byte_budget;
                stats.strategy_per_handle_byte_budget = strategy_cache_byte_budget;
            } else {
                debug_assert_eq!(
                    stats.metadata_per_handle_byte_budget,
                    metadata_cache_byte_budget
                );
                debug_assert_eq!(
                    stats.strategy_per_handle_byte_budget,
                    strategy_cache_byte_budget
                );
            }
            stats.metadata.merge(handle.service.metadata_cache_stats());
            stats
                .strategies
                .merge(handle.service.strategy_cache_stats());
        }
        stats
    }

    fn with_service<T>(
        &self,
        dimension: &DimensionRef,
        query: impl FnOnce(&V3QueryService) -> Result<T, ToolError>,
    ) -> Result<T, ToolError> {
        let archive_dir = self.archive_dirs.get(dimension).ok_or_else(|| {
            ToolError::new(
                "DIMENSION_NOT_FOUND",
                format!(
                    "V3 storage does not contain dimension {}",
                    dimension_key(dimension)
                ),
            )
        })?;
        let mut handles = self.handles.lock().expect("V3 handle pool lock poisoned");
        let service = handles.get_or_open(dimension, archive_dir, &self.options)?;
        drop(handles);
        query(&service)
    }
}

struct HandlePool {
    capacity: usize,
    counter: u64,
    entries: HashMap<DimensionRef, OpenHandle>,
    stats: HandlePoolStats,
}

struct OpenHandle {
    service: Arc<V3QueryService>,
    last_access: u64,
}

impl HandlePool {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            counter: 0,
            entries: HashMap::new(),
            stats: HandlePoolStats::default(),
        }
    }

    fn get_or_open(
        &mut self,
        dimension: &DimensionRef,
        archive_dir: &Path,
        options: &V3FacadeOptions,
    ) -> Result<Arc<V3QueryService>, ToolError> {
        self.counter = self.counter.wrapping_add(1);
        let sequence = self.counter;
        if self.entries.contains_key(dimension) {
            self.stats.hits = self.stats.hits.wrapping_add(1);
            let handle = self.entries.get_mut(dimension).expect("existing V3 handle");
            handle.last_access = sequence;
            return Ok(Arc::clone(&handle.service));
        }
        if self.entries.len() >= self.capacity {
            if let Some(lru_key) = self
                .entries
                .iter()
                .min_by_key(|(_, handle)| handle.last_access)
                .map(|(key, _)| key.clone())
            {
                let evicted = self.entries.remove(&lru_key).expect("existing V3 handle");
                evicted.service.resize_cache_budgets(0, 0);
                self.stats.evictions = self.stats.evictions.wrapping_add(1);
                self.resize_open_handle_caches(options);
            }
        }
        let next_handle_count = self.entries.len() + 1;
        let service = Arc::new(V3QueryService::open_with_options(
            archive_dir,
            V3ArchiveOpenOptions {
                verify_file_checksums: options.verify_file_checksums,
                metadata_cache_byte_budget: per_handle_budget(
                    options.metadata_cache_byte_budget,
                    next_handle_count,
                ),
                strategy_cache_byte_budget: per_handle_budget(
                    options.strategy_cache_byte_budget,
                    next_handle_count,
                ),
            },
        )?);
        self.entries.insert(
            dimension.clone(),
            OpenHandle {
                service: Arc::clone(&service),
                last_access: sequence,
            },
        );
        self.resize_open_handle_caches(options);
        self.stats.opens = self.stats.opens.wrapping_add(1);
        Ok(service)
    }

    fn resize_open_handle_caches(&self, options: &V3FacadeOptions) {
        let open_handle_count = self.entries.len();
        let metadata_cache_byte_budget =
            per_handle_budget(options.metadata_cache_byte_budget, open_handle_count);
        let strategy_cache_byte_budget =
            per_handle_budget(options.strategy_cache_byte_budget, open_handle_count);
        for handle in self.entries.values() {
            handle
                .service
                .resize_cache_budgets(metadata_cache_byte_budget, strategy_cache_byte_budget);
        }
    }
}

fn per_handle_budget(total_budget: usize, open_handle_count: usize) -> usize {
    total_budget / open_handle_count.max(1)
}
