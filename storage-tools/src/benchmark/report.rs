//! Main benchmark report model and renderer.
//!
//! Used by hot Binary, SQLite baseline, metadata, and native benchmark runners.
//! The hot compare runner reads this shape as input, but renders its own compare
//! report through `benchmark::compare::report`. Cold-start reports use
//! `benchmark::cold::report` and `benchmark::cold::compare`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::benchmark::hot::result_verifier::ResultVerificationSummary;
use crate::benchmark::memory_snapshot::BenchmarkMemoryReport;
use crate::benchmark::metrics::{BenchmarkCaseResult, BenchmarkTotals};
use crate::benchmark::report_support::{
    format_binary_bytes, format_ms, write_json_report, write_markdown_report,
};
use crate::benchmark::types::{BenchmarkWorkload, WorkloadMode, WorkloadSource};
use crate::errors::ToolError;

pub use crate::benchmark::report_support::generated_at_utc;

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
    pub hands_by_actions_queries: usize,
    pub drill_scenario_queries: usize,
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
    build_benchmark_report_for_engine(input, "binary")
}

pub fn build_benchmark_report_for_engine(input: ReportInput, engine: &str) -> BenchmarkRunReport {
    BenchmarkRunReport {
        generated_at: generated_at_utc(),
        engine: engine.to_owned(),
        source_db_path: input.source_db_path,
        binary_dir: input.binary_dir,
        meta_db_path: input.meta_db_path,
        options: input.options,
        workload: BenchmarkWorkloadSummary {
            dimensions: input.workload.dimensions,
            hand_queries: input.workload.hand_queries.len(),
            batch_queries: input.workload.batch_queries.len(),
            batch_size: input.workload.batch_size,
            hands_by_actions_queries: input.workload.hands_by_actions_queries.len(),
            drill_scenario_queries: input.workload.drill_scenario_queries.len(),
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

pub fn write_benchmark_json(path: &Path, report: &BenchmarkRunReport) -> Result<(), ToolError> {
    write_json_report(path, report)
}

pub fn write_benchmark_markdown(path: &Path, report: &BenchmarkRunReport) -> Result<(), ToolError> {
    write_markdown_report(path, render_benchmark_markdown(report))
}

pub fn render_benchmark_markdown(report: &BenchmarkRunReport) -> String {
    let mut markdown = String::new();
    markdown.push_str(match report.engine.as_str() {
        "sqlite" => "# SQLite Baseline Benchmark Report\n\n",
        "bun-native" => "# Bun Native SDK Benchmark Report\n\n",
        "drill-metadata" => "# Drill Metadata Benchmark Report\n\n",
        _ => "# Range Strata Binary Benchmark Report\n\n",
    });
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str("## Summary\n\n");
    markdown.push_str(&format!("- Engine: {}\n", report.engine));
    markdown.push_str(&format!("- Source SQLite: `{}`\n", report.source_db_path));
    if report.engine != "sqlite" {
        markdown.push_str(&format!("- Binary directory: `{}`\n", report.binary_dir));
        markdown.push_str(&format!("- meta.db: `{}`\n", report.meta_db_path));
    }
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
    markdown.push_str(&format!("- Result count: {}\n", report.totals.result_count));
    if report.cold_start.is_some() {
        markdown.push_str("- Cold start: measured in this report\n\n");
    } else if report.engine == "sqlite" {
        markdown.push_str("- Cold start: not measured by this command\n\n");
    } else {
        markdown.push_str("- Cold start: not measured by this command; use `benchmark-cold`\n\n");
    }

    markdown.push_str("## Cold Start\n\n");
    if let Some(cold_start) = &report.cold_start {
        let json =
            serde_json::to_string_pretty(cold_start).unwrap_or_else(|_| cold_start.to_string());
        markdown.push_str("```json\n");
        markdown.push_str(&json);
        markdown.push_str("\n```\n\n");
    } else {
        markdown.push_str("- Not measured by this command\n\n");
    }

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
        "- Hands-by-actions queries: {}\n",
        report.workload.hands_by_actions_queries
    ));
    markdown.push_str(&format!(
        "- Drill scenario metadata queries: {}\n",
        report.workload.drill_scenario_queries
    ));
    markdown.push_str(&format!(
        "- Warmup iterations: {}\n\n",
        report.options.warmup_iterations
    ));

    markdown.push_str("## Latency Results\n\n");
    markdown.push_str(
        "| case | iters | avg | p50 | p90 | p95 | p99 | max | qps | errors | resultCount |\n",
    );
    markdown.push_str(
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n",
    );
    for case in &report.cases {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {:.2} | {} | {} |\n",
            case.name,
            case.iterations,
            format_ms(case.avg_ms),
            format_ms(case.p50_ms),
            format_ms(case.p90_ms),
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

fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(format_binary_bytes)
        .unwrap_or_else(|| "unknown".to_owned())
}

fn format_optional_signed_bytes(value: Option<i64>) -> String {
    match value {
        Some(value) if value < 0 => format!("-{}", format_binary_bytes(value.unsigned_abs())),
        Some(value) => format_binary_bytes(value as u64),
        None => "unknown".to_owned(),
    }
}
