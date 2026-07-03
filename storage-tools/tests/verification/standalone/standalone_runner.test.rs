#[path = "../../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use std::fs;

use poker_hands_storage_tools::verification::report::VerifyLayer;
use poker_hands_storage_tools::verification::standalone::{
    run_standalone_verify, StandaloneVerifyOptions,
};
use range_store_core::sqlite::{Connection, Value};

use verify_store_fixture::build_verify_fixture;

#[test]
fn standalone_verify_passes_clean_build_output() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: true,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.manifest_ok);
    assert!(report.totals.catalog_ok);
    assert_eq!(report.totals.index_files_failed, 0);
    assert_eq!(report.totals.pack_files_failed, 0);
    assert_eq!(report.failures.len(), 0);
}

#[test]
fn standalone_verify_reports_missing_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    fs::remove_file(output_path.join("manifest.json")).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(!report.totals.manifest_ok);
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "MISSING_FILE"));
}

#[test]
fn standalone_verify_reports_corrupt_idx_magic() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    raw[0] = 0;
    fs::write(&idx_path, raw).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.index_files_failed > 0);
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "INVALID_MAGIC"));
}

#[test]
fn standalone_verify_reports_corrupt_bin_header() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let bin_path = output_path.join("ranges_default_6max_100BB.bin");
    let mut raw = fs::read(&bin_path).unwrap();
    raw[0] = 0;
    fs::write(&bin_path, raw).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report.totals.pack_files_failed > 0);
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::PackHeader));
}

#[test]
fn standalone_verify_reports_bad_action_schema_checksum() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let meta = Connection::open(&output_path.join("meta.db"), false).unwrap();
    meta.execute(
        "UPDATE action_schemas SET checksum = ?1 WHERE id = (
           SELECT id FROM action_schemas ORDER BY id LIMIT 1
         )",
        &[Value::from(0u32)],
    )
    .unwrap();
    drop(meta);

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report
        .failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::Catalog
            && failure.reason == "CHECKSUM_MISMATCH"));
}

#[test]
fn standalone_verify_reports_idx_out_of_order() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    let record0 = 16;
    let record1 = 16 + 22;
    let first = raw[record0..record0 + 4].to_vec();
    let second = raw[record1..record1 + 4].to_vec();
    raw[record0..record0 + 4].copy_from_slice(&second);
    raw[record1..record1 + 4].copy_from_slice(&first);
    fs::write(&idx_path, raw).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "OUT_OF_ORDER"));
}

#[test]
fn standalone_verify_reports_non_dense_idx_concrete_line_ids() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    let second_record = 16 + 22;
    raw[second_record..second_record + 4].copy_from_slice(&3u32.to_le_bytes());
    fs::write(&idx_path, raw).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report
        .failures
        .iter()
        .any(|failure| failure.reason == "NON_DENSE_CONCRETE_LINE_ID"));
}

#[test]
fn standalone_verify_reports_pack_size_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    raw[16 + 14..16 + 18].copy_from_slice(&1u32.to_le_bytes());
    fs::write(&idx_path, raw).unwrap();

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(report
        .failures
        .iter()
        .any(|failure| failure.layer == VerifyLayer::IndexPackCross
            && failure.reason == "PACK_SIZE_MISMATCH"));
    assert!(report.totals.index_pack_cross_failures > 0);
}
