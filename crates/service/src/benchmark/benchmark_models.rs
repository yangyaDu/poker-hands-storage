use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::domain::dimension::{dimension_key, DimensionRef};
use crate::errors::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkloadMode {
    #[serde(rename = "random")]
    Random,
    #[serde(rename = "abstract-local")]
    AbstractLocal,
}

impl WorkloadMode {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "random" => Ok(Self::Random),
            "abstract-local" => Ok(Self::AbstractLocal),
            _ => Err(AppError::invalid_argument(format!(
                "Invalid --workload-mode value: {value}. Use random or abstract-local."
            ))),
        }
    }
}

impl fmt::Display for WorkloadMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Random => f.write_str("random"),
            Self::AbstractLocal => f.write_str("abstract-local"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkCommand {
    pub source: PathBuf,
    pub dir: PathBuf,
    pub meta: PathBuf,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
    pub workload_path: Option<PathBuf>,
    pub seed: u64,
    pub hand_iterations: usize,
    pub batch_iterations: usize,
    pub batch_size: usize,
    pub batch_sizes: Vec<usize>,
    pub requested_dimensions: Vec<DimensionRef>,
    pub requested_dimension_values: Vec<String>,
    pub workload_mode: WorkloadMode,
    pub warmup_iterations: usize,
    pub verify_checksums: bool,
    pub verify_results: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandBenchmarkItem {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub hole_cards: String,
}

impl HandBenchmarkItem {
    pub fn dimension(&self) -> DimensionRef {
        DimensionRef::new(self.strategy.clone(), self.player_count, self.depth_bb)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchBenchmarkRequest {
    pub concrete_line_id: u32,
    pub hole_cards: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BatchBenchmarkItem {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub requests: Vec<BatchBenchmarkRequest>,
}

impl BatchBenchmarkItem {
    pub fn dimension(&self) -> DimensionRef {
        DimensionRef::new(self.strategy.clone(), self.player_count, self.depth_bb)
    }
}

pub type BatchQueriesBySize = Vec<(usize, Vec<BatchBenchmarkItem>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkWorkload {
    pub seed: u64,
    pub mode: WorkloadMode,
    pub dimensions: Vec<String>,
    pub hand_queries: Vec<HandBenchmarkItem>,
    pub batch_queries: Vec<BatchBenchmarkItem>,
    pub batch_size: usize,
    pub batch_queries_by_size: BatchQueriesBySize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadOptions {
    pub source_db_path: PathBuf,
    pub requested_dimensions: Vec<DimensionRef>,
    pub seed: u64,
    pub hand_iterations: usize,
    pub batch_iterations: usize,
    pub batch_size: usize,
    pub batch_sizes: Vec<usize>,
    pub workload_mode: WorkloadMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadSource {
    Generated,
    Loaded,
}

impl fmt::Display for WorkloadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generated => f.write_str("generated"),
            Self::Loaded => f.write_str("loaded"),
        }
    }
}

pub fn normalize_batch_sizes(batch_size: usize, batch_sizes: &[usize]) -> Vec<usize> {
    let mut sizes = if batch_sizes.is_empty() {
        vec![batch_size.max(1)]
    } else {
        batch_sizes
            .iter()
            .copied()
            .map(|size| size.max(1))
            .collect()
    };
    sizes.push(batch_size.max(1));
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

pub fn range_table_name(dimension: &DimensionRef) -> String {
    format!(
        "range_data_{}_{}max_{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

pub fn concrete_lines_table_name(dimension: &DimensionRef) -> String {
    format!(
        "concrete_lines_{}_{}max_{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

pub fn dimension_matches_requested(
    dimension: &DimensionRef,
    requested_dimensions: &[DimensionRef],
) -> bool {
    requested_dimensions.is_empty()
        || requested_dimensions
            .iter()
            .any(|requested| dimension_key(requested) == dimension_key(dimension))
}
