use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkCompareCommand {
    pub binary_report: PathBuf,
    pub sqlite_report: PathBuf,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
    pub allow_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCompareReport {
    pub generated_at: String,
    pub binary_report_path: String,
    pub sqlite_report_path: String,
    pub compatible_workload: bool,
    pub compatibility_notes: Vec<String>,
    pub cases: Vec<CaseComparison>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaseComparison {
    pub name: String,
    pub binary: CaseSide,
    pub sqlite: CaseSide,
    pub binary_to_sqlite_avg_latency_ratio: f64,
    pub binary_to_sqlite_p50_latency_ratio: f64,
    pub binary_to_sqlite_p95_latency_ratio: f64,
    pub binary_to_sqlite_p99_latency_ratio: f64,
    pub binary_to_sqlite_qps_ratio: f64,
    pub result_count_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaseSide {
    pub iterations: usize,
    pub avg_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub qps: f64,
    pub result_count: u64,
    pub error_count: u64,
}
