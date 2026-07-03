#[path = "../../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use std::fs;

use poker_hands_storage_tools::verification::cross::{run_cross_verify, CrossVerifyOptions};
use range_store_core::sqlite::{Connection, Value};

use verify_store_fixture::build_verify_fixture;

#[test]
fn cross_verify_sample_passes_clean_fixture() {
    let directory = tempfile::tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());

    let report = run_cross_verify(&CrossVerifyOptions {
        dir: output_path,
        source_db: source_path,
        sample_size: 100,
        max_failures: 50,
        verify_checksums: true,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.manifest_ok);
    assert!(report.totals.catalog_ok);
    assert_eq!(report.totals.failed_source_records, Some(0));
    assert_eq!(report.totals.extra_binary_records, Some(0));
    assert!(report.precision.is_some());
}

#[test]
fn cross_verify_full_detects_float32_mismatch_inside_legacy_tolerance() {
    let directory = tempfile::tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let source = Connection::open(&source_path, false).unwrap();
    source
        .execute(
            "UPDATE range_data_default_6max_100BB
             SET frequency = ?1
             WHERE concrete_line_id = 2 AND hole_cards = 'AKs'",
            &[Value::from(0.5000000596046448_f64)],
        )
        .unwrap();
    drop(source);

    let report = run_cross_verify(&CrossVerifyOptions {
        dir: output_path,
        source_db: source_path,
        sample_size: 0,
        max_failures: 50,
        verify_checksums: true,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.failed_source_records.unwrap_or_default() > 0);
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "FREQUENCY_FLOAT32_MISMATCH"));
    assert!(report.precision.as_ref().unwrap().frequency.mismatch_values > 0);
}

#[test]
fn cross_verify_full_counts_extra_binary_cells() {
    let directory = tempfile::tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let source = Connection::open(&source_path, false).unwrap();
    source
        .execute(
            "DELETE FROM range_data_default_6max_100BB
             WHERE concrete_line_id = 1 AND hole_cards = 'AA' AND action_name = 'raise'",
            &[],
        )
        .unwrap();
    drop(source);

    let report = run_cross_verify(&CrossVerifyOptions {
        dir: output_path,
        source_db: source_path,
        sample_size: 0,
        max_failures: 50,
        verify_checksums: true,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.extra_binary_records.unwrap_or_default() > 0);
}

#[test]
fn cross_verify_reports_non_dense_idx_lookup_layout() {
    let directory = tempfile::tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    let second_record = 16 + 22;
    raw[second_record..second_record + 4].copy_from_slice(&3u32.to_le_bytes());
    fs::write(&idx_path, raw).unwrap();

    let report = run_cross_verify(&CrossVerifyOptions {
        dir: output_path,
        source_db: source_path,
        sample_size: 0,
        max_failures: 50,
        verify_checksums: true,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "NON_DENSE_CONCRETE_LINE_ID"));
    assert!(report.failures.iter().any(|failure| {
        failure.reason == "IO_ERROR"
            && failure
                .message
                .contains("concreteLineId must be contiguous")
    }));
}

#[test]
fn cross_verify_writes_reports() {
    let directory = tempfile::tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let out_path = directory.path().join("cross.json");
    let md_path = directory.path().join("cross.md");

    run_cross_verify(&CrossVerifyOptions {
        dir: output_path,
        source_db: source_path,
        sample_size: 0,
        max_failures: 50,
        verify_checksums: false,
        out_path: Some(out_path.clone()),
        md_path: Some(md_path.clone()),
    })
    .unwrap();

    assert!(out_path.is_file());
    assert!(md_path.is_file());
    assert!(fs::read_to_string(md_path)
        .unwrap()
        .contains("Range Strata Binary Integrity Report"));
}
