use serde::{Deserialize, Serialize};

use crate::benchmark::memory_snapshot::MemorySnapshot;
use crate::benchmark::metrics;

// ---------------------------------------------------------------------------
// Cold-worker output (stdout JSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdWorkerOutput {
    pub ok: bool,
    pub store_open_and_first_query_ms: f64,
    pub result_count: usize,
    pub memory_before: MemorySnapshot,
    pub memory_after: MemorySnapshot,
    pub timings: ColdWorkerTimings,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdWorkerTimings {
    pub service_open_ms: f64,
    pub dimension_prewarm_ms: f64,
    pub first_query_ms: f64,
    pub close_ms: f64,
    pub worker_total_ms: f64,
}

// ---------------------------------------------------------------------------
// Cache eviction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColdStartMode {
    ProcessCold,
    OsBestEffort,
    LinuxDropCache,
}

impl ColdStartMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "process-cold" => Ok(Self::ProcessCold),
            "os-best-effort" => Ok(Self::OsBestEffort),
            "linux-drop-cache" => Ok(Self::LinuxDropCache),
            _ => Err(format!(
                "Invalid --mode value: {value}. Use process-cold, os-best-effort, or linux-drop-cache."
            )),
        }
    }
}

impl std::fmt::Display for ColdStartMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessCold => f.write_str("process-cold"),
            Self::OsBestEffort => f.write_str("os-best-effort"),
            Self::LinuxDropCache => f.write_str("linux-drop-cache"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvictionResult {
    pub requested: bool,
    pub method: ColdStartMode,
    pub succeeded: bool,
    pub duration_ms: f64,
    pub filler_size_bytes: u64,
    pub dataset_size_bytes: u64,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Query policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryPolicy {
    First,
    Fixed,
}

impl QueryPolicy {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "first" => Ok(Self::First),
            "fixed" => Ok(Self::Fixed),
            _ => Err(format!(
                "Invalid --query-policy value: {value}. Use first or fixed."
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Dimension query target
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DimensionQuery {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub hand: String,
}

// ---------------------------------------------------------------------------
// Run result (parent-side)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartRunResult {
    pub ok: bool,
    pub run_index: usize,
    pub store_open_and_first_query_ms: f64,
    pub result_count: usize,
    pub process_elapsed_ms: f64,
    pub process_overhead_ms: f64,
    pub memory_before: MemorySnapshot,
    pub memory_after: MemorySnapshot,
    pub timings: ColdWorkerTimings,
    pub eviction: EvictionResult,
    pub exit_code: i32,
    pub valid_json: bool,
    pub phase_accounting: PhaseAccounting,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhaseAccounting {
    pub phase_sum_ms: f64,
    pub worker_total_ms: f64,
    pub unaccounted_ms: f64,
    pub unaccounted_ratio: f64,
}

impl PhaseAccounting {
    pub fn compute(timings: &ColdWorkerTimings) -> Self {
        let phase_sum_ms = timings.service_open_ms
            + timings.dimension_prewarm_ms
            + timings.first_query_ms
            + timings.close_ms;
        let worker_total_ms = timings.worker_total_ms;
        let unaccounted_ms = worker_total_ms - phase_sum_ms;
        let unaccounted_ratio = if worker_total_ms > 0.0 {
            unaccounted_ms.abs() / worker_total_ms
        } else {
            0.0
        };
        Self {
            phase_sum_ms,
            worker_total_ms,
            unaccounted_ms,
            unaccounted_ratio,
        }
    }
}

// ---------------------------------------------------------------------------
// Latency summary (reused for cold-start reports)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub avg_ms: f64,
}

impl LatencySummary {
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                min_ms: 0.0,
                p50_ms: 0.0,
                p95_ms: 0.0,
                max_ms: 0.0,
                avg_ms: 0.0,
            };
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let total: f64 = sorted.iter().sum();
        Self {
            min_ms: sorted[0],
            p50_ms: metrics::percentile(&sorted, 50.0),
            p95_ms: metrics::percentile(&sorted, 95.0),
            max_ms: sorted[sorted.len() - 1],
            avg_ms: total / sorted.len() as f64,
        }
    }
}

// ---------------------------------------------------------------------------
// Run failure summary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartRunFailure {
    pub dimension: String,
    pub run_index: usize,
    pub exit_code: i32,
    pub error: String,
    pub valid_json: bool,
}

// ---------------------------------------------------------------------------
// Phase summaries
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartPhaseSummaries {
    pub service_open_ms: LatencySummary,
    pub dimension_prewarm_ms: LatencySummary,
    pub first_query_ms: LatencySummary,
    pub close_ms: LatencySummary,
    pub worker_total_ms: LatencySummary,
    pub process_overhead_ms: LatencySummary,
}

impl ColdStartPhaseSummaries {
    pub fn from_results(results: &[ColdStartRunResult]) -> Self {
        Self {
            service_open_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.timings.service_open_ms)
                    .collect::<Vec<_>>(),
            ),
            dimension_prewarm_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.timings.dimension_prewarm_ms)
                    .collect::<Vec<_>>(),
            ),
            first_query_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.timings.first_query_ms)
                    .collect::<Vec<_>>(),
            ),
            close_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.timings.close_ms)
                    .collect::<Vec<_>>(),
            ),
            worker_total_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.timings.worker_total_ms)
                    .collect::<Vec<_>>(),
            ),
            process_overhead_ms: LatencySummary::from_values(
                &results
                    .iter()
                    .map(|r| r.process_overhead_ms)
                    .collect::<Vec<_>>(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Dimension report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DimensionColdStartReport {
    pub dimension: String,
    pub query: DimensionQuery,
    pub runs: usize,
    pub success_count: usize,
    pub error_count: usize,
    pub store_open_and_first_query_ms: LatencySummary,
    pub process_elapsed_ms: LatencySummary,
    pub phase_timings: ColdStartPhaseSummaries,
    pub memory_delta_rss_bytes: LatencySummary,
    pub phase_accounting: PhaseAccounting,
    pub failures: Vec<ColdStartRunFailure>,
}

// ---------------------------------------------------------------------------
// Aggregate report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AggregateReport {
    pub dimensions: usize,
    pub runs: usize,
    pub successful_runs: usize,
    pub error_count: usize,
    pub store_open_and_first_query_ms: LatencySummary,
    pub process_elapsed_ms: LatencySummary,
    pub phase_timings: ColdStartPhaseSummaries,
    pub phase_accounting: PhaseAccounting,
    pub failures: Vec<ColdStartRunFailure>,
}

// ---------------------------------------------------------------------------
// Top-level report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartBenchmarkReport {
    pub generated_at: String,
    pub engine: String,
    pub mode: String,
    pub platform: String,
    pub runs_per_dimension: usize,
    pub source_db_path: String,
    pub binary_dir: String,
    pub meta_db_path: String,
    pub verify_checksums: bool,
    pub cache_filler_size_bytes: u64,
    pub dimensions: Vec<DimensionColdStartReport>,
    pub aggregate: AggregateReport,
    pub notes: Vec<String>,
}

impl ColdStartBenchmarkReport {
    pub fn has_errors(&self) -> bool {
        self.aggregate.error_count > 0
    }
}

// ---------------------------------------------------------------------------
// CLI command
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkColdCommand {
    pub source: std::path::PathBuf,
    pub dir: std::path::PathBuf,
    pub meta: std::path::PathBuf,
    pub out_path: std::path::PathBuf,
    pub md_path: std::path::PathBuf,
    pub mode: ColdStartMode,
    pub runs_per_dimension: usize,
    pub requested_dimensions: Vec<crate::domain::dimension::DimensionRef>,
    pub query_policy: QueryPolicy,
    pub fixed_concrete_line_id: Option<u32>,
    pub fixed_hand: Option<String>,
    pub cache_filler_mb: u64,
    pub max_errors_per_dimension: usize,
    pub fail_fast: bool,
    pub verify_checksums: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkSqliteColdCommand {
    pub source: std::path::PathBuf,
    pub dir: std::path::PathBuf,
    pub out_path: std::path::PathBuf,
    pub md_path: std::path::PathBuf,
    pub mode: ColdStartMode,
    pub runs_per_dimension: usize,
    pub requested_dimensions: Vec<crate::domain::dimension::DimensionRef>,
    pub query_policy: QueryPolicy,
    pub fixed_concrete_line_id: Option<u32>,
    pub fixed_hand: Option<String>,
    pub cache_filler_mb: u64,
    pub max_errors_per_dimension: usize,
    pub fail_fast: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkColdCompareCommand {
    pub binary_report: std::path::PathBuf,
    pub sqlite_report: std::path::PathBuf,
    pub out_path: std::path::PathBuf,
    pub md_path: std::path::PathBuf,
    pub allow_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartCompareReport {
    pub generated_at: String,
    pub binary_report_path: String,
    pub sqlite_report_path: String,
    pub compatible: bool,
    pub compatibility_notes: Vec<String>,
    pub aggregate: ColdStartComparison,
    pub dimensions: Vec<ColdStartComparison>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartComparison {
    pub name: String,
    pub binary: ColdStartComparisonSide,
    pub sqlite: ColdStartComparisonSide,
    pub process_elapsed_p50_ratio: f64,
    pub process_elapsed_p95_ratio: f64,
    pub store_open_and_first_query_p50_ratio: f64,
    pub store_open_and_first_query_p95_ratio: f64,
    pub first_query_p50_ratio: f64,
    pub first_query_p95_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ColdStartComparisonSide {
    pub runs: usize,
    pub successful_runs: usize,
    pub error_count: usize,
    pub process_elapsed_p50_ms: f64,
    pub process_elapsed_p95_ms: f64,
    pub store_open_and_first_query_p50_ms: f64,
    pub store_open_and_first_query_p95_ms: f64,
    pub worker_total_p50_ms: f64,
    pub worker_total_p95_ms: f64,
    pub first_query_p50_ms: f64,
    pub first_query_p95_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_worker_output_serde_round_trip() {
        let output = ColdWorkerOutput {
            ok: true,
            store_open_and_first_query_ms: 12.5,
            result_count: 3,
            memory_before: MemorySnapshot {
                rss_bytes: Some(1000),
                heap_total_bytes: None,
                heap_used_bytes: None,
                external_bytes: None,
                array_buffers_bytes: None,
                note: None,
            },
            memory_after: MemorySnapshot {
                rss_bytes: Some(2000),
                heap_total_bytes: None,
                heap_used_bytes: None,
                external_bytes: None,
                array_buffers_bytes: None,
                note: None,
            },
            timings: ColdWorkerTimings {
                service_open_ms: 3.0,
                dimension_prewarm_ms: 5.0,
                first_query_ms: 4.0,
                close_ms: 0.5,
                worker_total_ms: 12.5,
            },
            error: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        let parsed: ColdWorkerOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(output, parsed);
    }

    #[test]
    fn phase_accounting_zero_total() {
        let timings = ColdWorkerTimings {
            service_open_ms: 0.0,
            dimension_prewarm_ms: 0.0,
            first_query_ms: 0.0,
            close_ms: 0.0,
            worker_total_ms: 0.0,
        };
        let accounting = PhaseAccounting::compute(&timings);
        assert_eq!(accounting.unaccounted_ratio, 0.0);
        assert_eq!(accounting.phase_sum_ms, 0.0);
    }

    #[test]
    fn phase_accounting_normal() {
        let timings = ColdWorkerTimings {
            service_open_ms: 3.0,
            dimension_prewarm_ms: 5.0,
            first_query_ms: 2.0,
            close_ms: 0.5,
            worker_total_ms: 11.0,
        };
        let accounting = PhaseAccounting::compute(&timings);
        assert!((accounting.phase_sum_ms - 10.5).abs() < 1e-6);
        assert!((accounting.unaccounted_ms - 0.5).abs() < 1e-6);
        assert!(accounting.unaccounted_ratio < 0.05);
    }

    #[test]
    fn latency_summary_empty() {
        let summary = LatencySummary::from_values(&[]);
        assert_eq!(summary.min_ms, 0.0);
        assert_eq!(summary.avg_ms, 0.0);
    }

    #[test]
    fn cold_start_mode_parse() {
        assert_eq!(
            ColdStartMode::parse("process-cold").unwrap(),
            ColdStartMode::ProcessCold
        );
        assert_eq!(
            ColdStartMode::parse("os-best-effort").unwrap(),
            ColdStartMode::OsBestEffort
        );
        assert_eq!(
            ColdStartMode::parse("linux-drop-cache").unwrap(),
            ColdStartMode::LinuxDropCache
        );
        assert!(ColdStartMode::parse("invalid").is_err());
    }

    #[test]
    fn query_policy_parse() {
        assert_eq!(QueryPolicy::parse("first").unwrap(), QueryPolicy::First);
        assert_eq!(QueryPolicy::parse("fixed").unwrap(), QueryPolicy::Fixed);
        assert!(QueryPolicy::parse("invalid").is_err());
    }
}
