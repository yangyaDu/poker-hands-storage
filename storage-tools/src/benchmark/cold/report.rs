use std::path::Path;

use crate::benchmark::report_support::{markdown_table, write_json_report, write_markdown_report};
use crate::errors::ToolError;

use super::types::{ColdStartBenchmarkReport, ColdStartPhaseSummaries, LatencySummary};

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

fn render_cold_start_markdown(report: &ColdStartBenchmarkReport) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!(
        "# {} Cold-Start Benchmark\n\n",
        engine_title(&report.engine)
    ));
    out.push_str(&format!("Generated: {}\n\n", report.generated_at));

    // Summary
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
        format_ms(report.aggregate.store_open_and_first_query_ms.p50_ms),
        format_ms(report.aggregate.store_open_and_first_query_ms.p95_ms)
    ));
    out.push_str(&format!(
        "- Aggregate process elapsed p50 / p95: {} / {}\n",
        format_ms(report.aggregate.process_elapsed_ms.p50_ms),
        format_ms(report.aggregate.process_elapsed_ms.p95_ms)
    ));
    out.push_str(&format!(
        "- Phase accounting (worst): unaccounted {} ({:.2}%)\n\n",
        format_ms(report.aggregate.phase_accounting.unaccounted_ms),
        report.aggregate.phase_accounting.unaccounted_ratio * 100.0
    ));

    // Aggregate Phase Breakdown
    out.push_str("## Aggregate Phase Breakdown\n\n");
    let phase_rows = phase_summary_rows(&report.aggregate.phase_timings);
    out.push_str(&markdown_table(
        &["Phase", "P50", "P95", "Avg", "Max"],
        &phase_rows,
    ));
    out.push('\n');

    // Dimensions
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
        .map(|d| {
            vec![
                d.dimension.clone(),
                d.runs.to_string(),
                d.error_count.to_string(),
                format_ms(d.store_open_and_first_query_ms.p50_ms),
                format_ms(d.store_open_and_first_query_ms.p95_ms),
                format_ms(d.process_elapsed_ms.p50_ms),
                format_ms(d.process_elapsed_ms.p95_ms),
                format_bytes(d.memory_delta_rss_bytes.p95_ms),
                format!("{} / {}", d.query.concrete_line_id, d.query.hand),
            ]
        })
        .collect();
    out.push_str(&markdown_table(&dim_headers, &dim_rows));
    out.push('\n');

    // Failures
    out.push_str("## Failures\n\n");
    if report.aggregate.failures.is_empty() {
        out.push_str("None\n\n");
    } else {
        let fail_rows: Vec<Vec<String>> = report
            .aggregate
            .failures
            .iter()
            .map(|f| {
                vec![
                    f.dimension.clone(),
                    f.run_index.to_string(),
                    f.exit_code.to_string(),
                    f.valid_json.to_string(),
                    f.error.clone(),
                ]
            })
            .collect();
        out.push_str(&markdown_table(
            &["Dimension", "Run", "Exit Code", "Valid JSON", "Error"],
            &fail_rows,
        ));
        out.push('\n');
    }

    // Dimension Phase Breakdown
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
        .map(|d| {
            vec![
                d.dimension.clone(),
                format_ms(d.phase_timings.service_open_ms.p95_ms),
                format_ms(d.phase_timings.dimension_prewarm_ms.p95_ms),
                format_ms(d.phase_timings.first_query_ms.p95_ms),
                format_ms(d.phase_timings.worker_total_ms.p95_ms),
                format_ms(d.phase_timings.process_overhead_ms.p95_ms),
            ]
        })
        .collect();
    out.push_str(&markdown_table(&dim_phase_headers, &dim_phase_rows));
    out.push('\n');

    // Notes
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
        .map(|(name, s)| {
            vec![
                name.to_owned(),
                format_ms(s.p50_ms),
                format_ms(s.p95_ms),
                format_ms(s.avg_ms),
                format_ms(s.max_ms),
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

fn format_ms(value: f64) -> String {
    if !value.is_finite() {
        return "unknown".to_owned();
    }
    if value >= 10.0 {
        format!("{value:.2} ms")
    } else {
        format!("{value:.3} ms")
    }
}

fn format_bytes(value: f64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::cold::types::*;

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
    fn markdown_contains_sections() {
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
        let md = render_cold_start_markdown(&report);
        assert!(md.contains("## Summary"));
        assert!(md.contains("## Aggregate Phase Breakdown"));
        assert!(md.contains("## Dimensions"));
        assert!(md.contains("## Failures"));
        assert!(md.contains("None"));
        assert!(md.contains("## Notes"));
        assert!(md.contains("test note"));
    }

    #[test]
    fn format_ms_large() {
        assert_eq!(format_ms(12.345), "12.35 ms");
    }

    #[test]
    fn format_ms_small() {
        assert_eq!(format_ms(1.2345), "1.234 ms");
    }

    #[test]
    fn format_bytes_mb() {
        assert_eq!(format_bytes(2.5 * 1024.0 * 1024.0), "2.50 MB");
    }
}
