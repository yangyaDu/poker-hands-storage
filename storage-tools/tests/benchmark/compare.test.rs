use std::fs;

use poker_hands_storage_tools::benchmark::compare::run_benchmark_compare;
use poker_hands_storage_tools::benchmark::compare::types::BenchmarkCompareCommand;
use poker_hands_storage_tools::benchmark::memory_snapshot::{
    BenchmarkMemoryReport, MemorySnapshot,
};
use poker_hands_storage_tools::benchmark::metrics::{BenchmarkCaseResult, BenchmarkTotals};
use poker_hands_storage_tools::benchmark::report::{
    build_benchmark_report, build_benchmark_report_for_engine, BenchmarkOptionsSummary,
    BenchmarkRunReport, ReportInput,
};
use poker_hands_storage_tools::benchmark::types::{
    BenchmarkWorkload, WorkloadMode, WorkloadSource,
};
use tempfile::tempdir;

#[test]
fn compare_runner_writes_ratios_for_matching_reports() {
    let directory = tempdir().unwrap();
    let binary_path = directory.path().join("binary.json");
    let sqlite_path = directory.path().join("sqlite.json");
    let out_path = directory.path().join("compare.json");
    let md_path = directory.path().join("compare.md");

    write_report(&binary_path, &binary_report(4.0, 250.0, 4));
    write_report(&sqlite_path, &sqlite_report(2.0, 500.0, 4));

    let report = run_benchmark_compare(&BenchmarkCompareCommand {
        binary_report: binary_path,
        sqlite_report: sqlite_path,
        out_path: out_path.clone(),
        md_path: md_path.clone(),
        allow_mismatch: false,
    })
    .unwrap();

    assert!(report.compatible_workload);
    assert_eq!(report.cases.len(), 1);
    assert_eq!(report.cases[0].binary_to_sqlite_avg_latency_ratio, 2.0);
    assert_eq!(report.cases[0].binary_to_sqlite_qps_ratio, 0.5);
    assert!(report.cases[0].result_count_match);
    assert!(out_path.is_file());
    assert!(md_path.is_file());
}

#[test]
fn compare_runner_rejects_mismatched_workload_by_default() {
    let directory = tempdir().unwrap();
    let binary_path = directory.path().join("binary.json");
    let sqlite_path = directory.path().join("sqlite.json");

    write_report(&binary_path, &binary_report(4.0, 250.0, 4));
    let mut sqlite = sqlite_report(2.0, 500.0, 4);
    sqlite.workload.batch_size = 2;
    write_report(&sqlite_path, &sqlite);

    let error = run_benchmark_compare(&BenchmarkCompareCommand {
        binary_report: binary_path,
        sqlite_report: sqlite_path,
        out_path: directory.path().join("compare.json"),
        md_path: directory.path().join("compare.md"),
        allow_mismatch: false,
    })
    .unwrap_err();

    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("batch sizes differ"));
}

fn binary_report(avg_ms: f64, qps: f64, result_count: u64) -> BenchmarkRunReport {
    build_benchmark_report(report_input(avg_ms, qps, result_count))
}

fn sqlite_report(avg_ms: f64, qps: f64, result_count: u64) -> BenchmarkRunReport {
    build_benchmark_report_for_engine(report_input(avg_ms, qps, result_count), "sqlite")
}

fn report_input(avg_ms: f64, qps: f64, result_count: u64) -> ReportInput {
    ReportInput {
        source_db_path: "source.db".to_owned(),
        binary_dir: "range-strata".to_owned(),
        meta_db_path: "range-strata/meta.db".to_owned(),
        options: BenchmarkOptionsSummary {
            seed: 42,
            requested_dimensions: vec!["default:6:100".to_owned()],
            hand_iterations: 1,
            batch_iterations: 0,
            batch_size: 1,
            batch_sizes: vec![1],
            warmup_iterations: 0,
            verify_checksums: false,
            verify_results: false,
            workload_mode: WorkloadMode::Random,
        },
        workload: BenchmarkWorkload {
            seed: 42,
            mode: WorkloadMode::Random,
            dimensions: vec!["default:6max:100BB".to_owned()],
            hand_queries: Vec::new(),
            batch_queries: Vec::new(),
            batch_size: 1,
            batch_queries_by_size: Vec::new(),
        },
        workload_source: WorkloadSource::Loaded,
        workload_path: Some("workload.json".to_owned()),
        cases: vec![case(avg_ms, qps, result_count)],
        totals: BenchmarkTotals {
            iterations: 1,
            total_ms: avg_ms,
            avg_qps: qps,
            error_count: 0,
            result_count,
        },
        memory: BenchmarkMemoryReport::new(snapshot(), snapshot()),
        result_verification: None,
        notes: Vec::new(),
    }
}

fn case(avg_ms: f64, qps: f64, result_count: u64) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        name: "hand-strategy".to_owned(),
        description: "desc".to_owned(),
        iterations: 1,
        warmup_iterations: 0,
        total_ms: avg_ms,
        avg_ms,
        p50_ms: avg_ms,
        p95_ms: avg_ms,
        p99_ms: avg_ms,
        max_ms: avg_ms,
        qps,
        result_count,
        error_count: 0,
        first_error: None,
    }
}

fn snapshot() -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: None,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: None,
    }
}

fn write_report(path: &std::path::Path, report: &BenchmarkRunReport) {
    let json = serde_json::to_string_pretty(report).unwrap();
    fs::write(path, json).unwrap();
}
