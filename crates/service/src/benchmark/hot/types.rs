use std::path::PathBuf;

use crate::benchmark::types::WorkloadMode;
use crate::domain::dimension::DimensionRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkCommand {
    pub source: PathBuf,
    pub dir: PathBuf,
    pub meta: PathBuf,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
    pub workload_path: Option<PathBuf>,
    pub write_workload_path: Option<PathBuf>,
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
