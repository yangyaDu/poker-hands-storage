use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use range_store_core::DimensionReader;

use crate::errors::AppError;
use range_store_core::dimension::DimensionRef;
use range_store_core::manifest::QueryableDimension;

#[derive(Debug)]
pub struct HandlePool {
    data_dir: PathBuf,
    known: HashMap<String, QueryableDimension>,
    capacity: usize,
    state: RwLock<PoolState>,
}

#[derive(Debug, Default)]
struct PoolState {
    handles: HashMap<String, Arc<DimensionReader>>,
    lru: VecDeque<String>,
}

impl HandlePool {
    pub fn new(data_dir: PathBuf, dimensions: Vec<QueryableDimension>, capacity: usize) -> Self {
        let known = dimensions
            .into_iter()
            .map(|dimension| (dimension_key(&dimension), dimension))
            .collect();
        Self {
            data_dir,
            known,
            capacity: capacity.max(1),
            state: RwLock::new(PoolState::default()),
        }
    }

    pub fn get_or_open(&self, dimension: &DimensionRef) -> Result<Arc<DimensionReader>, AppError> {
        let key = dimension_key_ref(dimension);

        // Fast path: read lock for cached handles (no write contention)
        {
            let state = self
                .state
                .read()
                .map_err(|_| AppError::invalid_format("Dimension handle pool lock poisoned"))?;
            if let Some(reader) = state.handles.get(&key).cloned() {
                // LRU touch requires write lock, but skipping it on read path
                // is acceptable — the order is approximate and eviction is rare.
                return Ok(reader);
            }
        }

        // Slow path: write lock to open and cache a new handle
        let mut state = self
            .state
            .write()
            .map_err(|_| AppError::invalid_format("Dimension handle pool lock poisoned"))?;

        // Double-check after acquiring write lock (another thread may have opened it)
        if let Some(reader) = state.handles.get(&key).cloned() {
            touch(&mut state.lru, &key);
            return Ok(reader);
        }

        let known = self.known.get(&key).ok_or_else(|| {
            AppError::dimension_not_found(
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            )
        })?;
        let idx_path = self.data_dir.join(&known.idx_file);
        let bin_path = self.data_dir.join(&known.bin_file);
        let reader = Arc::new(DimensionReader::open(&idx_path, &bin_path).map_err(|_| {
            AppError::dimension_not_found(
                &dimension.strategy,
                dimension.player_count,
                dimension.depth_bb,
            )
        })?);

        state.handles.insert(key.clone(), Arc::clone(&reader));
        touch(&mut state.lru, &key);
        while state.lru.len() > self.capacity {
            if let Some(oldest) = state.lru.pop_front() {
                state.handles.remove(&oldest);
            }
        }
        Ok(reader)
    }

    pub fn open_count(&self) -> usize {
        self.state
            .read()
            .map(|state| state.handles.len())
            .unwrap_or_default()
    }

    pub fn known_dimensions(&self) -> Vec<String> {
        let mut dimensions: Vec<_> = self
            .known
            .values()
            .map(|dimension| {
                format!(
                    "{}_{}max_{}BB",
                    dimension.strategy, dimension.player_count, dimension.depth_bb
                )
            })
            .collect();
        dimensions.sort();
        dimensions
    }
}

fn touch(lru: &mut VecDeque<String>, key: &str) {
    if let Some(position) = lru.iter().position(|candidate| candidate == key) {
        lru.remove(position);
    }
    lru.push_back(key.to_owned());
}

fn dimension_key(dimension: &QueryableDimension) -> String {
    format!(
        "{}:{}:{}",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

fn dimension_key_ref(dimension: &DimensionRef) -> String {
    format!(
        "{}:{}:{}",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}
