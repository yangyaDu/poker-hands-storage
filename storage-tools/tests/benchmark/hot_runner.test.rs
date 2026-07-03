#[path = "../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use poker_hands_storage_tools::benchmark::hot::types::BenchmarkCommand;
use poker_hands_storage_tools::benchmark::run_hot_benchmark;
use poker_hands_storage_tools::benchmark::types::WorkloadMode;
use tempfile::tempdir;
use verify_store_fixture::build_verify_fixture;

#[test]
fn hot_runner_writes_reports_for_clean_fixture() {
    let directory = tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let report_path = directory.path().join("benchmark.json");
    let markdown_path = directory.path().join("benchmark.md");
    let workload_path = directory.path().join("workload.json");

    let command = BenchmarkCommand {
        source: source_path,
        dir: output_path.clone(),
        meta: output_path.join("meta.db"),
        out_path: report_path.clone(),
        md_path: markdown_path.clone(),
        workload_path: None,
        write_workload_path: Some(workload_path.clone()),
        seed: 11,
        hand_iterations: 3,
        batch_iterations: 2,
        batch_size: 1,
        batch_sizes: vec![1],
        requested_dimensions: Vec::new(),
        requested_dimension_values: Vec::new(),
        workload_mode: WorkloadMode::Random,
        warmup_iterations: 1,
        verify_checksums: true,
        verify_results: true,
    };

    let report = run_hot_benchmark(&command).unwrap();

    assert!(!report.has_errors());
    assert_eq!(report.cases.len(), 5);
    assert!(report.cases.iter().any(|case| case.name == "hand-strategy"));
    assert!(report
        .cases
        .iter()
        .any(|case| case.name == "batch-hand-strategy"));
    assert!(report.cases.iter().any(|case| case.name == "batch-size-1"));
    assert!(report
        .cases
        .iter()
        .any(|case| case.name == "hands-by-actions"));
    assert!(report
        .cases
        .iter()
        .any(|case| case.name == "drill-scenarios-metadata"));
    assert!(report.totals.result_count > 0);
    assert!(report_path.is_file());
    assert!(markdown_path.is_file());
    assert!(workload_path.is_file());
    assert_eq!(
        report.workload_path,
        Some(workload_path.display().to_string())
    );
}
