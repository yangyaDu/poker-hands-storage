use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use range_store_core::DimensionReader;

use crate::error::AppError;
use crate::manifest::QueryableDimension;
use crate::naming::DimensionRef;

#[derive(Debug)]
pub struct HandlePool {
    data_dir: PathBuf,
    known: HashMap<String, QueryableDimension>,
    capacity: usize,
    state: Mutex<PoolState>,
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
            state: Mutex::new(PoolState::default()),
        }
    }

    pub fn get_or_open(&self, dimension: &DimensionRef) -> Result<Arc<DimensionReader>, AppError> {
        let key = dimension_key_ref(dimension);
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::invalid_format("Dimension handle pool lock poisoned"))?;
        if let Some(reader) = state.handles.get(&key).cloned() {
            touch(&mut state.lru, &key);
            return Ok(reader);
        }

        let known = self.known.get(&key).ok_or_else(|| {
            AppError::bin_file_not_found(format!(
                "Unknown dimension {}:{}max:{}BB",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ))
        })?;
        let idx_path = self.data_dir.join(&known.idx_file);
        let bin_path = self.data_dir.join(&known.bin_file);
        let reader = Arc::new(
            DimensionReader::open(&idx_path, &bin_path).map_err(|error| {
                AppError::bin_file_not_found(format!(
                    "Failed to open {} and {}: {error}",
                    idx_path.display(),
                    bin_path.display()
                ))
            })?,
        );

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
            .lock()
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
