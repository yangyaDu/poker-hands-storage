use poker_hands_storage_tools::benchmark::hot::result_verifier::ResultVerificationSummary;
use poker_hands_storage_tools::benchmark::memory_snapshot::{
    BenchmarkMemoryReport, MemorySnapshot,
};
use poker_hands_storage_tools::benchmark::metrics::{BenchmarkCaseResult, BenchmarkTotals};
use poker_hands_storage_tools::benchmark::report::{
    build_benchmark_report, render_benchmark_markdown, BenchmarkOptionsSummary, ReportInput,
};
use poker_hands_storage_tools::benchmark::types::{
    BenchmarkWorkload, WorkloadMode, WorkloadSource,
};

#[test]
fn benchmark_report_renders_latency_memory_and_verification() {
    let report = build_benchmark_report(ReportInput {
        source_db_path: "source.db".to_owned(),
        binary_dir: "range-strata".to_owned(),
        meta_db_path: "range-strata/meta.db".to_owned(),
        options: BenchmarkOptionsSummary {
            seed: 42,
            requested_dimensions: vec!["default:6:100".to_owned()],
            hand_iterations: 2,
            batch_iterations: 1,
            batch_size: 1,
            batch_sizes: vec![1],
            warmup_iterations: 1,
            verify_checksums: false,
            verify_results: true,
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
            hands_by_actions_queries: Vec::new(),
            drill_scenario_queries: Vec::new(),
        },
        workload_source: WorkloadSource::Generated,
        workload_path: None,
        cases: vec![case("hand-strategy")],
        totals: BenchmarkTotals {
            iterations: 1,
            total_ms: 2.0,
            avg_qps: 500.0,
            error_count: 0,
            result_count: 2,
        },
        memory: BenchmarkMemoryReport::new(snapshot(100), snapshot(140)),
        result_verification: Some(ResultVerificationSummary {
            sample_size: 1,
            match_count: 0,
            mismatch_count: 1,
            error_count: 0,
            mismatches: vec!["default / 1 / AA: SQLite=3, rangeStrata=2".to_owned()],
            errors: Vec::new(),
        }),
        notes: vec!["note".to_owned()],
    });

    assert!(report.has_errors());
    let markdown = render_benchmark_markdown(&report);
    assert!(markdown.contains("hand-strategy"));
    assert!(markdown.contains("Result Verification"));
    assert!(markdown.contains("SQLite=3, rangeStrata=2"));
    assert!(markdown.contains("Hands-by-actions queries"));
    assert!(markdown.contains("Drill scenario metadata queries"));
    assert!(markdown.contains("Delta RSS"));
}

fn case(name: &str) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        name: name.to_owned(),
        description: "desc".to_owned(),
        iterations: 1,
        warmup_iterations: 0,
        total_ms: 2.0,
        avg_ms: 2.0,
        p50_ms: 2.0,
        p95_ms: 2.0,
        p99_ms: 2.0,
        max_ms: 2.0,
        qps: 500.0,
        result_count: 2,
        error_count: 0,
        first_error: None,
    }
}

fn snapshot(rss: u64) -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: Some(rss),
        heap_total_bytes: None,
        heap_used_bytes: Some(rss / 2),
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some("test".to_owned()),
    }
}
