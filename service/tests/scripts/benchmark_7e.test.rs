use std::path::PathBuf;

use poker_hands_storage_service::benchmark::cold::types::ColdStartMode;
use poker_hands_storage_service::benchmark::cold::{
    parse_benchmark_cold_compare_args, parse_benchmark_sqlite_cold_args,
};
use poker_hands_storage_service::benchmark::compare::parse_benchmark_compare_args;
use poker_hands_storage_service::benchmark::sqlite::parse_benchmark_sqlite_args;
use poker_hands_storage_service::benchmark::types::WorkloadMode;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn parse_benchmark_sqlite_args_uses_defaults() {
    let command = parse_benchmark_sqlite_args(args(&["--source", "data/sqlite/range.db"])).unwrap();

    assert_eq!(command.source, PathBuf::from("data/sqlite/range.db"));
    assert_eq!(
        command.out_path,
        PathBuf::from("reports/benchmark-sqlite.json")
    );
    assert_eq!(
        command.md_path,
        PathBuf::from("reports/benchmark-sqlite.md")
    );
    assert_eq!(command.seed, 42);
    assert_eq!(command.hand_iterations, 1000);
    assert_eq!(command.batch_iterations, 200);
    assert_eq!(command.batch_size, 20);
    assert_eq!(command.batch_sizes, vec![1, 5, 10, 20, 50, 100]);
    assert_eq!(command.workload_mode, WorkloadMode::Random);
}

#[test]
fn parse_benchmark_sqlite_args_accepts_explicit_options() {
    let command = parse_benchmark_sqlite_args(args(&[
        "--source",
        "source.db",
        "--out",
        "sqlite.json",
        "--md",
        "sqlite.md",
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
        "--workload-mode",
        "abstract-local",
        "--warmup-iterations",
        "2",
    ]))
    .unwrap();

    assert_eq!(command.out_path, PathBuf::from("sqlite.json"));
    assert_eq!(command.md_path, PathBuf::from("sqlite.md"));
    assert_eq!(command.workload_path, Some(PathBuf::from("workload.json")));
    assert_eq!(command.seed, 7);
    assert_eq!(command.hand_iterations, 11);
    assert_eq!(command.batch_iterations, 3);
    assert_eq!(command.batch_size, 4);
    assert_eq!(command.batch_sizes, vec![1, 4, 8]);
    assert_eq!(command.requested_dimensions.len(), 1);
    assert_eq!(command.workload_mode, WorkloadMode::AbstractLocal);
    assert_eq!(command.warmup_iterations, 2);
}

#[test]
fn parse_benchmark_sqlite_args_requires_source() {
    let error = parse_benchmark_sqlite_args(args(&[])).unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("--source is required"));
}

#[test]
fn parse_benchmark_compare_args_uses_defaults() {
    let command = parse_benchmark_compare_args(args(&[
        "--binary",
        "binary.json",
        "--sqlite",
        "sqlite.json",
    ]))
    .unwrap();

    assert_eq!(command.binary_report, PathBuf::from("binary.json"));
    assert_eq!(command.sqlite_report, PathBuf::from("sqlite.json"));
    assert_eq!(
        command.out_path,
        PathBuf::from("reports/benchmark-compare.json")
    );
    assert_eq!(
        command.md_path,
        PathBuf::from("reports/benchmark-compare.md")
    );
    assert!(!command.allow_mismatch);
}

#[test]
fn parse_benchmark_compare_args_accepts_explicit_options() {
    let command = parse_benchmark_compare_args(args(&[
        "--binary",
        "binary.json",
        "--sqlite",
        "sqlite.json",
        "--out",
        "compare.json",
        "--md",
        "compare.md",
        "--allow-mismatch",
    ]))
    .unwrap();

    assert_eq!(command.out_path, PathBuf::from("compare.json"));
    assert_eq!(command.md_path, PathBuf::from("compare.md"));
    assert!(command.allow_mismatch);
}

#[test]
fn parse_benchmark_sqlite_cold_args_uses_defaults() {
    let command = parse_benchmark_sqlite_cold_args(args(&[
        "--source",
        "data/sqlite/range.db",
        "--dir",
        "data/range-strata",
    ]))
    .unwrap();

    assert_eq!(command.source, PathBuf::from("data/sqlite/range.db"));
    assert_eq!(command.dir, PathBuf::from("data/range-strata"));
    assert_eq!(
        command.out_path,
        PathBuf::from("reports/benchmark-sqlite-cold-start.json")
    );
    assert_eq!(
        command.md_path,
        PathBuf::from("reports/benchmark-sqlite-cold-start.md")
    );
    assert_eq!(command.mode, ColdStartMode::ProcessCold);
    assert_eq!(command.runs_per_dimension, 10);
}

#[test]
fn parse_benchmark_sqlite_cold_args_requires_source_and_dir() {
    let error = parse_benchmark_sqlite_cold_args(args(&[])).unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("--source is required"));

    let error =
        parse_benchmark_sqlite_cold_args(args(&["--source", "data/sqlite/range.db"])).unwrap_err();
    assert_eq!(error.code(), "INVALID_ARGUMENT");
    assert!(error.message().contains("--dir is required"));
}

#[test]
fn parse_benchmark_cold_compare_args_uses_defaults() {
    let command = parse_benchmark_cold_compare_args(args(&[
        "--binary",
        "binary-cold.json",
        "--sqlite",
        "sqlite-cold.json",
    ]))
    .unwrap();

    assert_eq!(command.binary_report, PathBuf::from("binary-cold.json"));
    assert_eq!(command.sqlite_report, PathBuf::from("sqlite-cold.json"));
    assert_eq!(
        command.out_path,
        PathBuf::from("reports/benchmark-cold-compare.json")
    );
    assert_eq!(
        command.md_path,
        PathBuf::from("reports/benchmark-cold-compare.md")
    );
    assert!(!command.allow_mismatch);
}
