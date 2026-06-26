use std::collections::HashMap;
use std::fs;

use crate::benchmark::compare::report::{write_compare_json, write_compare_markdown};
use crate::benchmark::compare::types::{
    BenchmarkCompareCommand, BenchmarkCompareReport, CaseComparison, CaseSide,
};
use crate::benchmark::metrics::safe_ratio;
use crate::benchmark::report::{generated_at_utc, BenchmarkRunReport};
use crate::errors::AppError;

pub fn run_benchmark_compare(
    command: &BenchmarkCompareCommand,
) -> Result<BenchmarkCompareReport, AppError> {
    let binary = read_benchmark_report(&command.binary_report)?;
    let sqlite = read_benchmark_report(&command.sqlite_report)?;
    let mut compatibility_notes = compatibility_notes(&binary, &sqlite);
    if binary.engine != "binary" {
        compatibility_notes.push(format!(
            "Expected binary report engine to be `binary`, got `{}`.",
            binary.engine
        ));
    }
    if sqlite.engine != "sqlite" {
        compatibility_notes.push(format!(
            "Expected SQLite report engine to be `sqlite`, got `{}`.",
            sqlite.engine
        ));
    }
    let cases = compare_cases(&binary, &sqlite, &mut compatibility_notes);
    if !compatibility_notes.is_empty() && !command.allow_mismatch {
        return Err(AppError::invalid_argument(format!(
            "Benchmark reports are not comparable: {}. Use --allow-mismatch to write a warning report anyway.",
            compatibility_notes.join("; ")
        )));
    }

    let compatible_workload = compatibility_notes.is_empty();
    let report = BenchmarkCompareReport {
        generated_at: generated_at_utc(),
        binary_report_path: command.binary_report.display().to_string(),
        sqlite_report_path: command.sqlite_report.display().to_string(),
        compatible_workload,
        compatibility_notes,
        cases,
        notes: vec![
            "Latency ratios are binary / SQLite; values greater than 1.0 mean binary latency was higher.".to_owned(),
            "QPS ratio is binary / SQLite; values less than 1.0 mean binary throughput was lower.".to_owned(),
            "Cases with non-zero errors should be treated as failure data, not performance evidence.".to_owned(),
        ],
    };

    write_compare_json(&command.out_path, &report)?;
    write_compare_markdown(&command.md_path, &report)?;
    Ok(report)
}

fn read_benchmark_report(path: &std::path::Path) -> Result<BenchmarkRunReport, AppError> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|error| AppError::invalid_format(error.to_string()))
}

fn compatibility_notes(binary: &BenchmarkRunReport, sqlite: &BenchmarkRunReport) -> Vec<String> {
    let mut notes = Vec::new();
    if binary.workload.dimensions != sqlite.workload.dimensions {
        notes.push(format!(
            "dimensions differ: binary={:?}, sqlite={:?}",
            binary.workload.dimensions, sqlite.workload.dimensions
        ));
    }
    if binary.workload.hand_queries != sqlite.workload.hand_queries {
        notes.push(format!(
            "hand query counts differ: binary={}, sqlite={}",
            binary.workload.hand_queries, sqlite.workload.hand_queries
        ));
    }
    if binary.workload.batch_queries != sqlite.workload.batch_queries {
        notes.push(format!(
            "batch query counts differ: binary={}, sqlite={}",
            binary.workload.batch_queries, sqlite.workload.batch_queries
        ));
    }
    if binary.workload.batch_size != sqlite.workload.batch_size {
        notes.push(format!(
            "batch sizes differ: binary={}, sqlite={}",
            binary.workload.batch_size, sqlite.workload.batch_size
        ));
    }
    if binary.options.workload_mode != sqlite.options.workload_mode {
        notes.push(format!(
            "workload modes differ: binary={}, sqlite={}",
            binary.options.workload_mode, sqlite.options.workload_mode
        ));
    }
    notes
}

fn compare_cases(
    binary: &BenchmarkRunReport,
    sqlite: &BenchmarkRunReport,
    notes: &mut Vec<String>,
) -> Vec<CaseComparison> {
    let sqlite_cases = sqlite
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case))
        .collect::<HashMap<_, _>>();
    let mut comparisons = Vec::new();

    for binary_case in &binary.cases {
        let Some(sqlite_case) = sqlite_cases.get(binary_case.name.as_str()) else {
            notes.push(format!(
                "SQLite report is missing case `{}`.",
                binary_case.name
            ));
            continue;
        };
        if binary_case.iterations != sqlite_case.iterations {
            notes.push(format!(
                "case `{}` iterations differ: binary={}, sqlite={}",
                binary_case.name, binary_case.iterations, sqlite_case.iterations
            ));
        }
        comparisons.push(CaseComparison {
            name: binary_case.name.clone(),
            binary: CaseSide::from(binary_case),
            sqlite: CaseSide::from(*sqlite_case),
            binary_to_sqlite_avg_latency_ratio: safe_ratio(binary_case.avg_ms, sqlite_case.avg_ms),
            binary_to_sqlite_p50_latency_ratio: safe_ratio(binary_case.p50_ms, sqlite_case.p50_ms),
            binary_to_sqlite_p95_latency_ratio: safe_ratio(binary_case.p95_ms, sqlite_case.p95_ms),
            binary_to_sqlite_p99_latency_ratio: safe_ratio(binary_case.p99_ms, sqlite_case.p99_ms),
            binary_to_sqlite_qps_ratio: safe_ratio(binary_case.qps, sqlite_case.qps),
            result_count_match: binary_case.result_count == sqlite_case.result_count,
        });
    }

    for sqlite_case in &sqlite.cases {
        if !binary
            .cases
            .iter()
            .any(|case| case.name == sqlite_case.name)
        {
            notes.push(format!(
                "Binary report is missing case `{}`.",
                sqlite_case.name
            ));
        }
    }

    comparisons
}

impl From<&crate::benchmark::metrics::BenchmarkCaseResult> for CaseSide {
    fn from(case: &crate::benchmark::metrics::BenchmarkCaseResult) -> Self {
        Self {
            iterations: case.iterations,
            avg_ms: case.avg_ms,
            p50_ms: case.p50_ms,
            p95_ms: case.p95_ms,
            p99_ms: case.p99_ms,
            qps: case.qps,
            result_count: case.result_count,
            error_count: case.error_count,
        }
    }
}
