use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use range_store_core::dimension::{dimension_key, DimensionRef};
use range_store_core::query::{ActionFilter, QueryBatchResult, QueryResult};

use crate::errors::ToolError;

use super::line_matrix_store::{read_compact_archive_dimension, CompactArchiveOpenOptions};
use super::query_service::ProtoRangeQueryService;

const MATRIX_CACHE_CAPACITY_PER_HANDLE: usize = 1024;

pub struct ProtoRangeStoreFacade {
    archive_dirs: BTreeMap<String, PathBuf>,
    verify_checksums: bool,
    handles: Mutex<HandlePool>,
}

impl ProtoRangeStoreFacade {
    pub fn open(
        root_dir: impl AsRef<Path>,
        max_open_handles: usize,
        verify_checksums: bool,
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
            verify_checksums,
            handles: Mutex::new(HandlePool::new(max_open_handles)),
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

    pub fn prewarm(&self, dimension: &DimensionRef) -> Result<(), ToolError> {
        self.with_service(dimension, |_| Ok(()))
    }

    pub fn matrix_count(&self, dimension: &DimensionRef) -> Result<u64, ToolError> {
        self.with_service(dimension, |service| service.matrix_count(dimension))
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
        let service = handles.get_or_open(&key, archive_dir, self.verify_checksums)?;
        query(service)
    }
}

struct HandlePool {
    capacity: usize,
    counter: u64,
    entries: HashMap<String, (ProtoRangeQueryService, u64)>,
}

impl HandlePool {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            counter: 0,
            entries: HashMap::new(),
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get_or_open(
        &mut self,
        key: &str,
        archive_dir: &Path,
        verify_checksums: bool,
    ) -> Result<&ProtoRangeQueryService, ToolError> {
        self.counter = self.counter.wrapping_add(1);
        let access_sequence = self.counter;
        if self.entries.contains_key(key) {
            self.entries.get_mut(key).expect("existing Proto handle").1 = access_sequence;
        } else {
            let service = ProtoRangeQueryService::open_with_options(
                archive_dir,
                CompactArchiveOpenOptions {
                    verify_checksums,
                    cache_capacity: MATRIX_CACHE_CAPACITY_PER_HANDLE,
                },
            )?;
            if self.entries.len() >= self.capacity {
                if let Some(lru_key) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, (_, last_access))| *last_access)
                    .map(|(key, _)| key.clone())
                {
                    self.entries.remove(&lru_key);
                }
            }
            self.entries
                .insert(key.to_owned(), (service, access_sequence));
        }
        Ok(&self.entries.get(key).expect("inserted Proto handle").0)
    }
}
