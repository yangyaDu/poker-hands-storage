use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::benchmark::benchmark_models::{BenchmarkWorkload, WorkloadMode, WorkloadSource};
use crate::benchmark::memory_snapshot::BenchmarkMemoryReport;
use crate::benchmark::metrics::{BenchmarkCaseResult, BenchmarkTotals};
use crate::benchmark::result_verifier::ResultVerificationSummary;
use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkOptionsSummary {
    pub seed: u64,
    pub requested_dimensions: Vec<String>,
    pub hand_iterations: usize,
    pub batch_iterations: usize,
    pub batch_size: usize,
    pub batch_sizes: Vec<usize>,
    pub warmup_iterations: usize,
    pub verify_checksums: bool,
    pub verify_results: bool,
    pub workload_mode: WorkloadMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkWorkloadSummary {
    pub dimensions: Vec<String>,
    pub hand_queries: usize,
    pub batch_queries: usize,
    pub batch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRunReport {
    pub generated_at: String,
    pub engine: String,
    pub source_db_path: String,
    pub binary_dir: String,
    pub meta_db_path: String,
    pub options: BenchmarkOptionsSummary,
    pub workload: BenchmarkWorkloadSummary,
    pub workload_source: String,
    pub workload_path: Option<String>,
    pub cold_start: Option<serde_json::Value>,
    pub cases: Vec<BenchmarkCaseResult>,
    pub totals: BenchmarkTotals,
    pub memory: BenchmarkMemoryReport,
    pub result_verification: Option<ResultVerificationSummary>,
    pub notes: Vec<String>,
}

impl BenchmarkRunReport {
    pub fn has_errors(&self) -> bool {
        self.totals.error_count > 0
            || self
                .result_verification
                .as_ref()
                .is_some_and(ResultVerificationSummary::has_errors)
    }
}

pub struct ReportInput {
    pub source_db_path: String,
    pub binary_dir: String,
    pub meta_db_path: String,
    pub options: BenchmarkOptionsSummary,
    pub workload: BenchmarkWorkload,
    pub workload_source: WorkloadSource,
    pub workload_path: Option<String>,
    pub cases: Vec<BenchmarkCaseResult>,
    pub totals: BenchmarkTotals,
    pub memory: BenchmarkMemoryReport,
    pub result_verification: Option<ResultVerificationSummary>,
    pub notes: Vec<String>,
}

pub fn build_benchmark_report(input: ReportInput) -> BenchmarkRunReport {
    BenchmarkRunReport {
        generated_at: generated_at_utc(),
        engine: "binary".to_owned(),
        source_db_path: input.source_db_path,
        binary_dir: input.binary_dir,
        meta_db_path: input.meta_db_path,
        options: input.options,
        workload: BenchmarkWorkloadSummary {
            dimensions: input.workload.dimensions,
            hand_queries: input.workload.hand_queries.len(),
            batch_queries: input.workload.batch_queries.len(),
            batch_size: input.workload.batch_size,
        },
        workload_source: input.workload_source.to_string(),
        workload_path: input.workload_path,
        cold_start: None,
        cases: input.cases,
        totals: input.totals,
        memory: input.memory,
        result_verification: input.result_verification,
        notes: input.notes,
    }
}

pub fn write_benchmark_json(path: &Path, report: &BenchmarkRunReport) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|error| AppError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn write_benchmark_markdown(path: &Path, report: &BenchmarkRunReport) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, render_benchmark_markdown(report))?;
    Ok(())
}

pub fn render_benchmark_markdown(report: &BenchmarkRunReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Range Strata Binary Benchmark Report\n\n");
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str("## Summary\n\n");
    markdown.push_str(&format!("- Engine: {}\n", report.engine));
    markdown.push_str(&format!("- Source SQLite: `{}`\n", report.source_db_path));
    markdown.push_str(&format!("- Binary directory: `{}`\n", report.binary_dir));
    markdown.push_str(&format!("- meta.db: `{}`\n", report.meta_db_path));
    markdown.push_str(&format!(
        "- Dimensions: {}\n",
        report.workload.dimensions.join(", ")
    ));
    markdown.push_str(&format!("- Workload seed: {}\n", report.options.seed));
    markdown.push_str(&format!(
        "- Total iterations: {}\n",
        report.totals.iterations
    ));
    markdown.push_str(&format!(
        "- Total elapsed: {}\n",
        format_ms(report.totals.total_ms)
    ));
    markdown.push_str(&format!("- Aggregate QPS: {:.2}\n", report.totals.avg_qps));
    markdown.push_str(&format!("- Error count: {}\n", report.totals.error_count));
    markdown.push_str(&format!(
        "- Result action count: {}\n",
        report.totals.result_count
    ));
    markdown.push_str("- Cold start: not measured by this command; use `benchmark-cold`\n\n");

    markdown.push_str("## Workload\n\n");
    markdown.push_str(&format!("- Source: {}\n", report.workload_source));
    if let Some(path) = &report.workload_path {
        markdown.push_str(&format!("- Workload path: `{path}`\n"));
    }
    markdown.push_str(&format!("- Mode: {}\n", report.options.workload_mode));
    markdown.push_str(&format!(
        "- Hand queries: {}\n",
        report.workload.hand_queries
    ));
    markdown.push_str(&format!(
        "- Batch queries: {}\n",
        report.workload.batch_queries
    ));
    markdown.push_str(&format!("- Batch size: {}\n", report.workload.batch_size));
    markdown.push_str(&format!(
        "- Warmup iterations: {}\n\n",
        report.options.warmup_iterations
    ));

    markdown.push_str("## Latency Results\n\n");
    markdown
        .push_str("| case | iters | avg | p50 | p95 | p99 | max | qps | errors | resultCount |\n");
    markdown.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for case in &report.cases {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {:.2} | {} | {} |\n",
            case.name,
            case.iterations,
            format_ms(case.avg_ms),
            format_ms(case.p50_ms),
            format_ms(case.p95_ms),
            format_ms(case.p99_ms),
            format_ms(case.max_ms),
            case.qps,
            case.error_count,
            case.result_count
        ));
    }
    markdown.push('\n');

    markdown.push_str("## Memory\n\n");
    markdown.push_str(&format!(
        "- Before RSS: {}\n",
        format_optional_bytes(report.memory.before.rss_bytes)
    ));
    markdown.push_str(&format!(
        "- After RSS: {}\n",
        format_optional_bytes(report.memory.after.rss_bytes)
    ));
    markdown.push_str(&format!(
        "- Delta RSS: {}\n",
        format_optional_signed_bytes(report.memory.delta_rss_bytes)
    ));
    markdown.push_str(&format!(
        "- Before heap approximation: {}\n",
        format_optional_bytes(report.memory.before.heap_used_bytes)
    ));
    markdown.push_str(&format!(
        "- After heap approximation: {}\n",
        format_optional_bytes(report.memory.after.heap_used_bytes)
    ));
    for note in &report.memory.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown.push('\n');

    markdown.push_str("## Result Verification\n\n");
    if let Some(verification) = &report.result_verification {
        markdown.push_str(&format!(
            "- Sample size: {}\n- Match: {}\n- Mismatch: {}\n- Errors: {}\n",
            verification.sample_size,
            verification.match_count,
            verification.mismatch_count,
            verification.error_count
        ));
        if !verification.mismatches.is_empty() {
            markdown.push_str("- First mismatches:\n");
            for mismatch in &verification.mismatches {
                markdown.push_str(&format!("  - {mismatch}\n"));
            }
        }
        if !verification.errors.is_empty() {
            markdown.push_str("- First verification errors:\n");
            for error in &verification.errors {
                markdown.push_str(&format!("  - {error}\n"));
            }
        }
    } else {
        markdown.push_str("- Not requested\n");
    }
    markdown.push('\n');

    markdown.push_str("## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn format_ms(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_owned();
    }
    if value >= 1000.0 {
        format!("{:.2} s", value / 1000.0)
    } else if value >= 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.3} ms")
    }
}

fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(format_bytes)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn format_optional_signed_bytes(value: Option<i64>) -> String {
    match value {
        Some(value) if value < 0 => format!("-{}", format_bytes(value.unsigned_abs())),
        Some(value) => format_bytes(value as u64),
        None => "unknown".to_owned(),
    }
}

fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut unit_index = 0;
    let mut size = value as f64;
    while size >= 1024.0 && unit_index + 1 < UNITS.len() {
        size /= 1024.0;
        unit_index += 1;
    }
    if unit_index == 0 {
        format!("{value} {}", UNITS[unit_index])
    } else {
        format!("{size:.2} {}", UNITS[unit_index])
    }
}

fn generated_at_utc() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = duration.as_secs() as i64;
    let millis = duration.subsec_millis();
    let (year, month, day, hour, minute, second) = unix_seconds_to_utc(seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn unix_seconds_to_utc(seconds: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let hour = (seconds_of_day / 3_600) as u32;
    let minute = ((seconds_of_day % 3_600) / 60) as u32;
    let second = (seconds_of_day % 60) as u32;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, hour, minute, second)
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}
