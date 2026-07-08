//! Benchmark report models and renderers.
//!
//! This is the single report module for hot Binary, SQLite baseline, metadata,
//! native, hot compare, cold-start, and cold-start compare benchmark reports.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::benchmark::cold::types::{
    ColdStartBenchmarkReport, ColdStartCompareReport, ColdStartComparison, ColdStartPhaseSummaries,
    LatencySummary,
};
use crate::benchmark::hot::result_verifier::ResultVerificationSummary;
use crate::benchmark::hot::types::BenchmarkCompareReport;
use crate::benchmark::memory_snapshot::BenchmarkMemoryReport;
use crate::benchmark::metrics::{BenchmarkCaseResult, BenchmarkTotals};
use crate::benchmark::report_support::{
    format_binary_bytes, format_ms, markdown_table, write_json_report, write_markdown_report,
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

pub fn write_compare_json(path: &Path, report: &BenchmarkCompareReport) -> Result<(), ToolError> {
    write_json_report(path, report)
}

pub fn write_compare_markdown(
    path: &Path,
    report: &BenchmarkCompareReport,
) -> Result<(), ToolError> {
    write_markdown_report(path, render_compare_markdown(report))
}

pub fn render_compare_markdown(report: &BenchmarkCompareReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Range Strata Binary vs SQLite Benchmark Compare\n\n");
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str("## Summary\n\n");
    markdown.push_str(&format!(
        "- Binary report: `{}`\n",
        report.binary_report_path
    ));
    markdown.push_str(&format!(
        "- SQLite report: `{}`\n",
        report.sqlite_report_path
    ));
    markdown.push_str(&format!(
        "- Compatible workload: {}\n\n",
        report.compatible_workload
    ));

    if !report.compatibility_notes.is_empty() {
        markdown.push_str("## Compatibility Notes\n\n");
        for note in &report.compatibility_notes {
            markdown.push_str(&format!("- {note}\n"));
        }
        markdown.push('\n');
    }

    markdown.push_str("## Case Comparison\n\n");
    markdown.push_str("| case | binary avg | sqlite avg | latency ratio | binary p95 | sqlite p95 | p95 ratio | binary qps | sqlite qps | qps ratio | errors | result match |\n");
    markdown.push_str(
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n",
    );
    for case in &report.cases {
        markdown.push_str(&format!(
            "| {} | {} | {} | {:.3} | {} | {} | {:.3} | {:.2} | {:.2} | {:.3} | {}/{} | {} |\n",
            case.name,
            format_ms(case.binary.avg_ms),
            format_ms(case.sqlite.avg_ms),
            case.binary_to_sqlite_avg_latency_ratio,
            format_ms(case.binary.p95_ms),
            format_ms(case.sqlite.p95_ms),
            case.binary_to_sqlite_p95_latency_ratio,
            case.binary.qps,
            case.sqlite.qps,
            case.binary_to_sqlite_qps_ratio,
            case.binary.error_count,
            case.sqlite.error_count,
            case.result_count_match
        ));
    }
    markdown.push('\n');

    markdown.push_str("## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

pub fn write_cold_start_json(
    path: &Path,
    report: &ColdStartBenchmarkReport,
) -> Result<(), ToolError> {
    write_json_report(path, report)
}

pub fn write_cold_start_markdown(
    path: &Path,
    report: &ColdStartBenchmarkReport,
) -> Result<(), ToolError> {
    write_markdown_report(path, render_cold_start_markdown(report))
}

pub fn render_cold_start_markdown(report: &ColdStartBenchmarkReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "# {} Cold-Start Benchmark\n\n",
        engine_title(&report.engine)
    ));
    out.push_str(&format!("Generated: {}\n\n", report.generated_at));

    out.push_str("## Summary\n\n");
    out.push_str(&format!("- Engine: {}\n", report.engine));
    out.push_str(&format!("- Mode: {}\n", report.mode));
    out.push_str(&format!("- Platform: {}\n", report.platform));
    out.push_str(&format!("- Source DB: `{}`\n", report.source_db_path));
    if report.engine == "binary" {
        out.push_str(&format!("- Binary dir: `{}`\n", report.binary_dir));
        out.push_str(&format!("- meta.db: `{}`\n", report.meta_db_path));
    }
    out.push_str(&format!("- Dimensions: {}\n", report.aggregate.dimensions));
    out.push_str(&format!(
        "- Runs per dimension: {}\n",
        report.runs_per_dimension
    ));
    out.push_str(&format!("- Total runs: {}\n", report.aggregate.runs));
    out.push_str(&format!("- Errors: {}\n", report.aggregate.error_count));
    out.push_str(&format!(
        "- Cache filler size: {:.1} MB\n",
        report.cache_filler_size_bytes as f64 / (1024.0 * 1024.0)
    ));
    out.push_str(&format!(
        "- Successful runs: {}\n",
        report.aggregate.successful_runs
    ));
    out.push_str(&format!(
        "- Aggregate store open + first query p50 / p95: {} / {}\n",
        format_cold_ms(report.aggregate.store_open_and_first_query_ms.p50_ms),
        format_cold_ms(report.aggregate.store_open_and_first_query_ms.p95_ms)
    ));
    out.push_str(&format!(
        "- Aggregate process elapsed p50 / p95: {} / {}\n",
        format_cold_ms(report.aggregate.process_elapsed_ms.p50_ms),
        format_cold_ms(report.aggregate.process_elapsed_ms.p95_ms)
    ));
    out.push_str(&format!(
        "- Phase accounting (worst): unaccounted {} ({:.2}%)\n\n",
        format_cold_ms(report.aggregate.phase_accounting.unaccounted_ms),
        report.aggregate.phase_accounting.unaccounted_ratio * 100.0
    ));

    out.push_str("## Aggregate Phase Breakdown\n\n");
    let phase_rows = phase_summary_rows(&report.aggregate.phase_timings);
    out.push_str(&markdown_table(
        &["Phase", "P50", "P95", "Avg", "Max"],
        &phase_rows,
    ));
    out.push('\n');

    out.push_str("## Dimensions\n\n");
    let dim_headers = [
        "Dimension",
        "Runs",
        "Errors",
        "Store+Query P50",
        "Store+Query P95",
        "Process P50",
        "Process P95",
        "RSS Delta P95",
        "Query",
    ];
    let dim_rows: Vec<Vec<String>> = report
        .dimensions
        .iter()
        .map(|dimension| {
            vec![
                dimension.dimension.clone(),
                dimension.runs.to_string(),
                dimension.error_count.to_string(),
                format_cold_ms(dimension.store_open_and_first_query_ms.p50_ms),
                format_cold_ms(dimension.store_open_and_first_query_ms.p95_ms),
                format_cold_ms(dimension.process_elapsed_ms.p50_ms),
                format_cold_ms(dimension.process_elapsed_ms.p95_ms),
                format_cold_bytes(dimension.memory_delta_rss_bytes.p95_ms),
                format!(
                    "{} / {}",
                    dimension.query.concrete_line_id, dimension.query.hand
                ),
            ]
        })
        .collect();
    out.push_str(&markdown_table(&dim_headers, &dim_rows));
    out.push('\n');

    out.push_str("## Failures\n\n");
    if report.aggregate.failures.is_empty() {
        out.push_str("None\n\n");
    } else {
        let fail_rows: Vec<Vec<String>> = report
            .aggregate
            .failures
            .iter()
            .map(|failure| {
                vec![
                    failure.dimension.clone(),
                    failure.run_index.to_string(),
                    failure.exit_code.to_string(),
                    failure.valid_json.to_string(),
                    failure.error.clone(),
                ]
            })
            .collect();
        out.push_str(&markdown_table(
            &["Dimension", "Run", "Exit Code", "Valid JSON", "Error"],
            &fail_rows,
        ));
        out.push('\n');
    }

    out.push_str("## Dimension Phase Breakdown\n\n");
    let dim_phase_headers = [
        "Dimension",
        "Service Open P95",
        "Prewarm P95",
        "Query P95",
        "Worker Total P95",
        "Process Overhead P95",
    ];
    let dim_phase_rows: Vec<Vec<String>> = report
        .dimensions
        .iter()
        .map(|dimension| {
            vec![
                dimension.dimension.clone(),
                format_cold_ms(dimension.phase_timings.service_open_ms.p95_ms),
                format_cold_ms(dimension.phase_timings.dimension_prewarm_ms.p95_ms),
                format_cold_ms(dimension.phase_timings.first_query_ms.p95_ms),
                format_cold_ms(dimension.phase_timings.worker_total_ms.p95_ms),
                format_cold_ms(dimension.phase_timings.process_overhead_ms.p95_ms),
            ]
        })
        .collect();
    out.push_str(&markdown_table(&dim_phase_headers, &dim_phase_rows));
    out.push('\n');

    out.push_str("## Notes\n\n");
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    out.push('\n');

    out
}

fn phase_summary_rows(summary: &ColdStartPhaseSummaries) -> Vec<Vec<String>> {
    let rows: Vec<(&str, &LatencySummary)> = vec![
        ("Service open (meta.db + schemas)", &summary.service_open_ms),
        (
            "Dimension prewarm (idx/bin mmap)",
            &summary.dimension_prewarm_ms,
        ),
        ("First query sync decode", &summary.first_query_ms),
        ("Service close", &summary.close_ms),
        ("Worker measured total", &summary.worker_total_ms),
        ("Parent process overhead", &summary.process_overhead_ms),
    ];

    rows.into_iter()
        .map(|(name, summary)| {
            vec![
                name.to_owned(),
                format_cold_ms(summary.p50_ms),
                format_cold_ms(summary.p95_ms),
                format_cold_ms(summary.avg_ms),
                format_cold_ms(summary.max_ms),
            ]
        })
        .collect()
}

fn engine_title(engine: &str) -> &str {
    match engine {
        "binary" => "Range Strata Binary",
        "sqlite" => "SQLite",
        _ => engine,
    }
}

fn format_cold_ms(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_owned();
    }
    if value >= 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.3} ms")
    }
}

fn format_cold_bytes(value: f64) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0 B".to_owned();
    }
    let abs = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    if abs >= 1024.0 * 1024.0 * 1024.0 {
        format!("{sign}{:.2} GB", abs / (1024.0 * 1024.0 * 1024.0))
    } else if abs >= 1024.0 * 1024.0 {
        format!("{sign}{:.2} MB", abs / (1024.0 * 1024.0))
    } else if abs >= 1024.0 {
        format!("{sign}{:.2} KB", abs / 1024.0)
    } else {
        format!("{sign}{:.0} B", abs)
    }
}

pub fn write_cold_compare_json(
    path: &Path,
    report: &ColdStartCompareReport,
) -> Result<(), ToolError> {
    write_json_report(path, report)
}

pub fn write_cold_compare_markdown(
    path: &Path,
    report: &ColdStartCompareReport,
) -> Result<(), ToolError> {
    write_markdown_report(path, render_cold_compare_markdown(report))
}

pub fn render_cold_compare_markdown(report: &ColdStartCompareReport) -> String {
    let mut out = String::new();
    out.push_str("# Range Strata Binary vs SQLite Cold-Start Compare\n\n");
    out.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- Binary report: `{}`\n",
        report.binary_report_path
    ));
    out.push_str(&format!(
        "- SQLite report: `{}`\n",
        report.sqlite_report_path
    ));
    out.push_str(&format!("- Compatible: {}\n\n", report.compatible));

    if !report.compatibility_notes.is_empty() {
        out.push_str("## Compatibility Notes\n\n");
        for note in &report.compatibility_notes {
            out.push_str(&format!("- {note}\n"));
        }
        out.push('\n');
    }

    out.push_str("## Aggregate Comparison\n\n");
    out.push_str(&cold_comparison_table(std::slice::from_ref(
        &report.aggregate,
    )));
    out.push('\n');

    out.push_str("## Dimension Comparison\n\n");
    out.push_str(&cold_comparison_table(&report.dimensions));
    out.push('\n');

    out.push_str("## Notes\n\n");
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    out
}

fn cold_comparison_table(rows: &[ColdStartComparison]) -> String {
    let mut out = String::new();
    out.push_str("| name | binary process p50 | sqlite process p50 | process p50 ratio | binary process p95 | sqlite process p95 | process p95 ratio | binary store+query p95 | sqlite store+query p95 | store+query p95 ratio | binary first-query p95 | sqlite first-query p95 | first-query p95 ratio | errors |\n");
    out.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for row in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {:.3} | {} | {} | {:.3} | {} | {} | {:.3} | {} | {} | {:.3} | {}/{} |\n",
            row.name,
            format_ms(row.binary.process_elapsed_p50_ms),
            format_ms(row.sqlite.process_elapsed_p50_ms),
            row.process_elapsed_p50_ratio,
            format_ms(row.binary.process_elapsed_p95_ms),
            format_ms(row.sqlite.process_elapsed_p95_ms),
            row.process_elapsed_p95_ratio,
            format_ms(row.binary.store_open_and_first_query_p95_ms),
            format_ms(row.sqlite.store_open_and_first_query_p95_ms),
            row.store_open_and_first_query_p95_ratio,
            format_ms(row.binary.first_query_p95_ms),
            format_ms(row.sqlite.first_query_p95_ms),
            row.first_query_p95_ratio,
            row.binary.error_count,
            row.sqlite.error_count,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::cold::types::{AggregateReport, PhaseAccounting};

    fn empty_latency() -> LatencySummary {
        LatencySummary::from_values(&[])
    }

    fn empty_phase() -> ColdStartPhaseSummaries {
        ColdStartPhaseSummaries {
            service_open_ms: empty_latency(),
            dimension_prewarm_ms: empty_latency(),
            first_query_ms: empty_latency(),
            close_ms: empty_latency(),
            worker_total_ms: empty_latency(),
            process_overhead_ms: empty_latency(),
        }
    }

    #[test]
    fn cold_start_markdown_contains_sections() {
        let report = ColdStartBenchmarkReport {
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
            engine: "binary".to_owned(),
            mode: "process-cold".to_owned(),
            platform: "windows".to_owned(),
            runs_per_dimension: 3,
            source_db_path: "test.db".to_owned(),
            binary_dir: "data/".to_owned(),
            meta_db_path: "data/meta.db".to_owned(),
            verify_checksums: false,
            cache_filler_size_bytes: 0,
            dimensions: vec![],
            aggregate: AggregateReport {
                dimensions: 0,
                runs: 0,
                successful_runs: 0,
                error_count: 0,
                store_open_and_first_query_ms: empty_latency(),
                process_elapsed_ms: empty_latency(),
                phase_timings: empty_phase(),
                phase_accounting: PhaseAccounting {
                    phase_sum_ms: 0.0,
                    worker_total_ms: 0.0,
                    unaccounted_ms: 0.0,
                    unaccounted_ratio: 0.0,
                },
                failures: vec![],
            },
            notes: vec!["test note".to_owned()],
        };
        let markdown = render_cold_start_markdown(&report);
        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("## Aggregate Phase Breakdown"));
        assert!(markdown.contains("## Dimensions"));
        assert!(markdown.contains("## Failures"));
        assert!(markdown.contains("None"));
        assert!(markdown.contains("## Notes"));
        assert!(markdown.contains("test note"));
    }

    #[test]
    fn format_cold_ms_large() {
        assert_eq!(format_cold_ms(12.345), "12.35 ms");
    }

    #[test]
    fn format_cold_ms_small() {
        assert_eq!(format_cold_ms(1.2345), "1.234 ms");
    }

    #[test]
    fn format_cold_bytes_mb() {
        assert_eq!(format_cold_bytes(2.5 * 1024.0 * 1024.0), "2.50 MB");
    }
}
