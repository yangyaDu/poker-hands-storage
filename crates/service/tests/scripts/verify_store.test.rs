use std::path::PathBuf;

use poker_hands_storage_service::verification::cli::parse_verify_args;
use poker_hands_storage_service::verification::report::VerifyMode;

#[test]
fn parse_verify_args_uses_standalone_report_defaults() {
    let command = parse_verify_args(vec![
        "--mode".to_owned(),
        "standalone".to_owned(),
        "--dir".to_owned(),
        "data/range-strata".to_owned(),
    ])
    .unwrap();

    assert_eq!(command.mode, VerifyMode::Standalone);
    assert_eq!(command.dir, PathBuf::from("data/range-strata"));
    assert_eq!(
        command.out_path,
        PathBuf::from("reports/range-strata-verify-standalone.json")
    );
    assert_eq!(
        command.md_path,
        PathBuf::from("reports/range-strata-verify-standalone.md")
    );
}

#[test]
fn parse_verify_args_requires_source_for_cross() {
    let error = parse_verify_args(vec![
        "--mode".to_owned(),
        "cross".to_owned(),
        "--dir".to_owned(),
        "data/range-strata".to_owned(),
    ])
    .unwrap_err();

    assert!(error.message().contains("--source is required"));
}

#[test]
fn parse_verify_args_rejects_invalid_mode() {
    let error = parse_verify_args(vec![
        "--mode".to_owned(),
        "bad".to_owned(),
        "--dir".to_owned(),
        "data/range-strata".to_owned(),
    ])
    .unwrap_err();

    assert!(error.message().contains("Invalid --mode"));
}

#[test]
fn parse_verify_args_accepts_zero_sample_size() {
    let command = parse_verify_args(vec![
        "--mode".to_owned(),
        "cross".to_owned(),
        "--dir".to_owned(),
        "data/range-strata".to_owned(),
        "--source".to_owned(),
        "data/sqlite/range.db".to_owned(),
        "--sample-size".to_owned(),
        "0".to_owned(),
    ])
    .unwrap();

    assert_eq!(command.mode, VerifyMode::Cross);
    assert_eq!(command.sample_size, 0);
}
