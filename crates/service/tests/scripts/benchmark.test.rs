use std::path::PathBuf;

use poker_hands_storage_service::benchmark::benchmark_models::WorkloadMode;
use poker_hands_storage_service::scripts::benchmark::{
    parse_benchmark_args, parse_requested_dimension,
};

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn parse_benchmark_args_uses_defaults() {
    let command = parse_benchmark_args(args(&[
        "--dir",
        "data/range-strata",
        "--source",
        "data/sqlite/range.db",
    ]))
    .unwrap();

    assert_eq!(command.source, PathBuf::from("data/sqlite/range.db"));
    assert_eq!(command.dir, PathBuf::from("data/range-strata"));
    assert_eq!(command.meta, PathBuf::from("data/range-strata/meta.db"));
    assert_eq!(command.seed, 42);
    assert_eq!(command.hand_iterations, 1000);
    assert_eq!(command.batch_iterations, 200);
    assert_eq!(command.batch_size, 20);
    assert_eq!(command.batch_sizes, vec![1, 5, 10, 20, 50, 100]);
    assert_eq!(command.workload_mode, WorkloadMode::Random);
    assert!(!command.verify_checksums);
    assert!(!command.verify_results);
}

#[test]
fn parse_benchmark_args_accepts_explicit_options() {
    let command = parse_benchmark_args(args(&[
        "--dir",
        "out",
        "--source",
        "source.db",
        "--meta",
        "meta.db",
        "--out",
        "report.json",
        "--md",
        "report.md",
        "--workload",
        "workload.json",
        "--seed",
        "7",
        "--iterations",
        "9",
        "--hand-iterations",
        "11",
        "--batch-iterations",
        "3",
        "--batch-size",
        "4",
        "--batch-sizes",
        "1,4,8",
        "--dimension",
        "default:6:100",
        "--dimension",
        "special_8max_40BB",
        "--workload-mode",
        "abstract-local",
        "--warmup-iterations",
        "2",
        "--verify-checksum",
        "--verify-results",
    ]))
    .unwrap();

    assert_eq!(command.meta, PathBuf::from("meta.db"));
    assert_eq!(command.out_path, PathBuf::from("report.json"));
    assert_eq!(command.md_path, PathBuf::from("report.md"));
    assert_eq!(command.workload_path, Some(PathBuf::from("workload.json")));
    assert_eq!(command.seed, 7);
    assert_eq!(command.hand_iterations, 11);
    assert_eq!(command.batch_iterations, 3);
    assert_eq!(command.batch_size, 4);
    assert_eq!(command.batch_sizes, vec![1, 4, 8]);
    assert_eq!(command.requested_dimensions.len(), 2);
    assert_eq!(command.workload_mode, WorkloadMode::AbstractLocal);
    assert_eq!(command.warmup_iterations, 2);
    assert!(command.verify_checksums);
    assert!(command.verify_results);
}

#[test]
fn parse_benchmark_args_requires_dir_and_source() {
    let error = parse_benchmark_args(args(&["--dir", "out"])).unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("--source is required"));

    let error = parse_benchmark_args(args(&["--source", "source.db"])).unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("--dir is required"));
}

#[test]
fn parse_benchmark_args_rejects_invalid_workload_mode() {
    let error = parse_benchmark_args(args(&[
        "--dir",
        "out",
        "--source",
        "source.db",
        "--workload-mode",
        "nearby",
    ]))
    .unwrap_err();

    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("random or abstract-local"));
}

#[test]
fn parse_requested_dimension_accepts_colon_and_table_forms() {
    let colon = parse_requested_dimension("default:6max:100BB").unwrap();
    assert_eq!(colon.strategy, "default");
    assert_eq!(colon.player_count, 6);
    assert_eq!(colon.depth_bb, 100);

    let table = parse_requested_dimension("special_strategy_8max_40BB").unwrap();
    assert_eq!(table.strategy, "special_strategy");
    assert_eq!(table.player_count, 8);
    assert_eq!(table.depth_bb, 40);
}
