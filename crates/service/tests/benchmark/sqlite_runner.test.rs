#[path = "../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use poker_hands_storage_service::benchmark::sqlite::run_sqlite_benchmark;
use poker_hands_storage_service::benchmark::sqlite::types::BenchmarkSqliteCommand;
use poker_hands_storage_service::benchmark::types::WorkloadMode;
use tempfile::tempdir;
use verify_store_fixture::build_verify_fixture;

#[test]
fn sqlite_runner_writes_reports_for_clean_fixture() {
    let directory = tempdir().unwrap();
    let (source_path, _output_path) = build_verify_fixture(directory.path());
    let report_path = directory.path().join("benchmark-sqlite.json");
    let markdown_path = directory.path().join("benchmark-sqlite.md");

    let command = BenchmarkSqliteCommand {
        source: source_path,
        out_path: report_path.clone(),
        md_path: markdown_path.clone(),
        workload_path: None,
        seed: 11,
        hand_iterations: 3,
        batch_iterations: 2,
        batch_size: 1,
        batch_sizes: vec![1],
        requested_dimensions: Vec::new(),
        requested_dimension_values: Vec::new(),
        workload_mode: WorkloadMode::Random,
        warmup_iterations: 1,
    };

    let report = run_sqlite_benchmark(&command).unwrap();

    assert_eq!(report.engine, "sqlite");
    assert!(!report.has_errors());
    assert_eq!(report.cases.len(), 3);
    assert!(report.cases.iter().any(|case| case.name == "hand-strategy"));
    assert!(report
        .cases
        .iter()
        .any(|case| case.name == "batch-hand-strategy"));
    assert!(report.cases.iter().any(|case| case.name == "batch-size-1"));
    assert!(report.totals.result_count > 0);
    assert!(report_path.is_file());
    assert!(markdown_path.is_file());
}
