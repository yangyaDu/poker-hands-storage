use std::fs;
use std::path::Path;

use crate::benchmark::metrics::safe_ratio;
use crate::benchmark::report_support::{format_ms, write_json_report, write_markdown_report};
use crate::errors::ToolError;

use super::types::{
    BenchmarkColdCompareCommand, ColdStartBenchmarkReport, ColdStartCompareReport,
    ColdStartComparison, ColdStartComparisonSide, DimensionColdStartReport,
};

pub fn run_cold_start_compare(
    command: &BenchmarkColdCompareCommand,
) -> Result<ColdStartCompareReport, ToolError> {
    let binary = read_cold_report(&command.binary_report)?;
    let sqlite = read_cold_report(&command.sqlite_report)?;
    let compatibility_notes = compatibility_notes(&binary, &sqlite);
    if !compatibility_notes.is_empty() && !command.allow_mismatch {
        return Err(ToolError::invalid_argument(format!(
            "Cold-start reports are not compatible: {}",
            compatibility_notes.join("; ")
        )));
    }

    let aggregate = ColdStartComparison {
        name: "aggregate".to_owned(),
        binary: aggregate_side(&binary),
        sqlite: aggregate_side(&sqlite),
        process_elapsed_p50_ratio: safe_ratio(
            binary.aggregate.process_elapsed_ms.p50_ms,
            sqlite.aggregate.process_elapsed_ms.p50_ms,
        ),
        process_elapsed_p95_ratio: safe_ratio(
            binary.aggregate.process_elapsed_ms.p95_ms,
            sqlite.aggregate.process_elapsed_ms.p95_ms,
        ),
        store_open_and_first_query_p50_ratio: safe_ratio(
            binary.aggregate.store_open_and_first_query_ms.p50_ms,
            sqlite.aggregate.store_open_and_first_query_ms.p50_ms,
        ),
        store_open_and_first_query_p95_ratio: safe_ratio(
            binary.aggregate.store_open_and_first_query_ms.p95_ms,
            sqlite.aggregate.store_open_and_first_query_ms.p95_ms,
        ),
        first_query_p50_ratio: safe_ratio(
            binary.aggregate.phase_timings.first_query_ms.p50_ms,
            sqlite.aggregate.phase_timings.first_query_ms.p50_ms,
        ),
        first_query_p95_ratio: safe_ratio(
            binary.aggregate.phase_timings.first_query_ms.p95_ms,
            sqlite.aggregate.phase_timings.first_query_ms.p95_ms,
        ),
    };

    let mut dimensions = Vec::new();
    for binary_dimension in &binary.dimensions {
        if let Some(sqlite_dimension) = sqlite
            .dimensions
            .iter()
            .find(|candidate| candidate.dimension == binary_dimension.dimension)
        {
            dimensions.push(dimension_comparison(binary_dimension, sqlite_dimension));
        }
    }

    let report = ColdStartCompareReport {
        generated_at: crate::benchmark::report::generated_at_utc(),
        binary_report_path: command.binary_report.display().to_string(),
        sqlite_report_path: command.sqlite_report.display().to_string(),
        compatible: compatibility_notes.is_empty(),
        compatibility_notes,
        aggregate,
        dimensions,
        notes: vec![
            "Ratios are binary / SQLite; values greater than 1.0 mean binary cold-start latency was higher.".to_owned(),
            "processElapsedMs is parent-observed end-to-end fresh-process time.".to_owned(),
            "storeOpenAndFirstQueryMs uses each engine's native open plus first query path. SQLite has no dimension prewarm phase.".to_owned(),
            "process-cold does not guarantee OS page cache eviction; compare only reports the current observed run.".to_owned(),
            "Rows with non-zero errors should be treated as failure data, not performance evidence.".to_owned(),
        ],
    };

    write_cold_compare_json(&command.out_path, &report)?;
    write_cold_compare_markdown(&command.md_path, &report)?;
    Ok(report)
}

fn read_cold_report(path: &Path) -> Result<ColdStartBenchmarkReport, ToolError> {
    let json = fs::read_to_string(path)?;
    serde_json::from_str(&json).map_err(|error| ToolError::invalid_format(error.to_string()))
}

fn compatibility_notes(
    binary: &ColdStartBenchmarkReport,
    sqlite: &ColdStartBenchmarkReport,
) -> Vec<String> {
    let mut notes = Vec::new();
    if binary.engine != "binary" {
        notes.push(format!("left report engine is {}", binary.engine));
    }
    if sqlite.engine != "sqlite" {
        notes.push(format!("right report engine is {}", sqlite.engine));
    }
    if binary.mode != sqlite.mode {
        notes.push(format!("mode differs: {} vs {}", binary.mode, sqlite.mode));
    }
    if binary.runs_per_dimension != sqlite.runs_per_dimension {
        notes.push(format!(
            "runs per dimension differ: {} vs {}",
            binary.runs_per_dimension, sqlite.runs_per_dimension
        ));
    }

    let binary_dimensions: Vec<&str> = binary
        .dimensions
        .iter()
        .map(|dimension| dimension.dimension.as_str())
        .collect();
    let sqlite_dimensions: Vec<&str> = sqlite
        .dimensions
        .iter()
        .map(|dimension| dimension.dimension.as_str())
        .collect();
    if binary_dimensions != sqlite_dimensions {
        notes.push("dimension list differs".to_owned());
    }

    for binary_dimension in &binary.dimensions {
        if let Some(sqlite_dimension) = sqlite
            .dimensions
            .iter()
            .find(|candidate| candidate.dimension == binary_dimension.dimension)
        {
            if binary_dimension.query != sqlite_dimension.query {
                notes.push(format!("query differs for {}", binary_dimension.dimension));
            }
        }
    }

    notes
}

fn dimension_comparison(
    binary: &DimensionColdStartReport,
    sqlite: &DimensionColdStartReport,
) -> ColdStartComparison {
    ColdStartComparison {
        name: binary.dimension.clone(),
        binary: dimension_side(binary),
        sqlite: dimension_side(sqlite),
        process_elapsed_p50_ratio: safe_ratio(
            binary.process_elapsed_ms.p50_ms,
            sqlite.process_elapsed_ms.p50_ms,
        ),
        process_elapsed_p95_ratio: safe_ratio(
            binary.process_elapsed_ms.p95_ms,
            sqlite.process_elapsed_ms.p95_ms,
        ),
        store_open_and_first_query_p50_ratio: safe_ratio(
            binary.store_open_and_first_query_ms.p50_ms,
            sqlite.store_open_and_first_query_ms.p50_ms,
        ),
        store_open_and_first_query_p95_ratio: safe_ratio(
            binary.store_open_and_first_query_ms.p95_ms,
            sqlite.store_open_and_first_query_ms.p95_ms,
        ),
        first_query_p50_ratio: safe_ratio(
            binary.phase_timings.first_query_ms.p50_ms,
            sqlite.phase_timings.first_query_ms.p50_ms,
        ),
        first_query_p95_ratio: safe_ratio(
            binary.phase_timings.first_query_ms.p95_ms,
            sqlite.phase_timings.first_query_ms.p95_ms,
        ),
    }
}

fn aggregate_side(report: &ColdStartBenchmarkReport) -> ColdStartComparisonSide {
    ColdStartComparisonSide {
        runs: report.aggregate.runs,
        successful_runs: report.aggregate.successful_runs,
        error_count: report.aggregate.error_count,
        process_elapsed_p50_ms: report.aggregate.process_elapsed_ms.p50_ms,
        process_elapsed_p95_ms: report.aggregate.process_elapsed_ms.p95_ms,
        store_open_and_first_query_p50_ms: report.aggregate.store_open_and_first_query_ms.p50_ms,
        store_open_and_first_query_p95_ms: report.aggregate.store_open_and_first_query_ms.p95_ms,
        worker_total_p50_ms: report.aggregate.phase_timings.worker_total_ms.p50_ms,
        worker_total_p95_ms: report.aggregate.phase_timings.worker_total_ms.p95_ms,
        first_query_p50_ms: report.aggregate.phase_timings.first_query_ms.p50_ms,
        first_query_p95_ms: report.aggregate.phase_timings.first_query_ms.p95_ms,
    }
}

fn dimension_side(report: &DimensionColdStartReport) -> ColdStartComparisonSide {
    ColdStartComparisonSide {
        runs: report.runs,
        successful_runs: report.success_count,
        error_count: report.error_count,
        process_elapsed_p50_ms: report.process_elapsed_ms.p50_ms,
        process_elapsed_p95_ms: report.process_elapsed_ms.p95_ms,
        store_open_and_first_query_p50_ms: report.store_open_and_first_query_ms.p50_ms,
        store_open_and_first_query_p95_ms: report.store_open_and_first_query_ms.p95_ms,
        worker_total_p50_ms: report.phase_timings.worker_total_ms.p50_ms,
        worker_total_p95_ms: report.phase_timings.worker_total_ms.p95_ms,
        first_query_p50_ms: report.phase_timings.first_query_ms.p50_ms,
        first_query_p95_ms: report.phase_timings.first_query_ms.p95_ms,
    }
}

fn write_cold_compare_json(path: &Path, report: &ColdStartCompareReport) -> Result<(), ToolError> {
    write_json_report(path, report)
}

fn write_cold_compare_markdown(
    path: &Path,
    report: &ColdStartCompareReport,
) -> Result<(), ToolError> {
    write_markdown_report(path, render_cold_compare_markdown(report))
}

fn render_cold_compare_markdown(report: &ColdStartCompareReport) -> String {
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
    out.push_str(&comparison_table(std::slice::from_ref(&report.aggregate)));
    out.push('\n');

    out.push_str("## Dimension Comparison\n\n");
    out.push_str(&comparison_table(&report.dimensions));
    out.push('\n');

    out.push_str("## Notes\n\n");
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    out
}

fn comparison_table(rows: &[ColdStartComparison]) -> String {
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
    use crate::benchmark::cold::types::{
        AggregateReport, ColdStartPhaseSummaries, DimensionQuery, LatencySummary, PhaseAccounting,
    };

    #[test]
    fn compatibility_accepts_matching_reports() {
        let binary = report("binary");
        let sqlite = report("sqlite");
        assert!(compatibility_notes(&binary, &sqlite).is_empty());
    }

    #[test]
    fn compatibility_rejects_wrong_engines() {
        let binary = report("sqlite");
        let sqlite = report("binary");
        let notes = compatibility_notes(&binary, &sqlite);
        assert!(notes.iter().any(|note| note.contains("left report engine")));
        assert!(notes
            .iter()
            .any(|note| note.contains("right report engine")));
    }

    fn report(engine: &str) -> ColdStartBenchmarkReport {
        let dimension = DimensionColdStartReport {
            dimension: "default:6:100".to_owned(),
            query: DimensionQuery {
                strategy: "default".to_owned(),
                player_count: 6,
                depth_bb: 100,
                concrete_line_id: 1,
                hand: "AA".to_owned(),
            },
            runs: 1,
            success_count: 1,
            error_count: 0,
            store_open_and_first_query_ms: latency(10.0),
            process_elapsed_ms: latency(12.0),
            phase_timings: phase(1.0),
            memory_delta_rss_bytes: latency(0.0),
            phase_accounting: accounting(),
            failures: vec![],
        };
        ColdStartBenchmarkReport {
            generated_at: "2026-01-01T00:00:00Z".to_owned(),
            engine: engine.to_owned(),
            mode: "process-cold".to_owned(),
            platform: "windows".to_owned(),
            runs_per_dimension: 1,
            source_db_path: "source.db".to_owned(),
            binary_dir: "data".to_owned(),
            meta_db_path: "data/meta.db".to_owned(),
            verify_checksums: false,
            cache_filler_size_bytes: 0,
            dimensions: vec![dimension],
            aggregate: AggregateReport {
                dimensions: 1,
                runs: 1,
                successful_runs: 1,
                error_count: 0,
                store_open_and_first_query_ms: latency(10.0),
                process_elapsed_ms: latency(12.0),
                phase_timings: phase(1.0),
                phase_accounting: accounting(),
                failures: vec![],
            },
            notes: vec![],
        }
    }

    fn latency(value: f64) -> LatencySummary {
        LatencySummary {
            min_ms: value,
            p50_ms: value,
            p95_ms: value,
            max_ms: value,
            avg_ms: value,
        }
    }

    fn phase(value: f64) -> ColdStartPhaseSummaries {
        ColdStartPhaseSummaries {
            service_open_ms: latency(value),
            dimension_prewarm_ms: latency(0.0),
            first_query_ms: latency(value),
            close_ms: latency(value),
            worker_total_ms: latency(value),
            process_overhead_ms: latency(value),
        }
    }

    fn accounting() -> PhaseAccounting {
        PhaseAccounting {
            phase_sum_ms: 0.0,
            worker_total_ms: 0.0,
            unaccounted_ms: 0.0,
            unaccounted_ratio: 0.0,
        }
    }
}
