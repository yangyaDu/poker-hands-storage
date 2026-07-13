#[path = "../support/verify_store_fixture.rs"]
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
fn standalone_verify_reports_missing_concrete_line_lookup_index() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let meta = Connection::open(&output_path.join("meta.db"), false).unwrap();
    meta.exec("DROP INDEX idx_concrete_lines_default_6max_100BB_concrete_line")
        .unwrap();
    drop(meta);

    let report = run_standalone_verify(&StandaloneVerifyOptions {
        dir: output_path,
        verify_checksums: false,
        out_path: None,
        md_path: None,
    })
    .unwrap();

    assert!(!report.totals.catalog_ok);
    assert!(report.failures.iter().any(|failure| {
        failure.layer == VerifyLayer::Catalog && failure.reason == "MISSING_INDEX"
    }));
}

#[test]
fn standalone_verify_reports_non_dense_metadata_concrete_line_ids() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let meta = Connection::open(&output_path.join("meta.db"), false).unwrap();
    meta.execute(
        "UPDATE \"concrete_lines_default_6max_100BB\"
         SET concrete_line_id = 3
         WHERE concrete_line_id = 2",
        &[],
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
        .any(|failure| failure.reason == "NON_DENSE_CONCRETE_LINE_ID"));
}
#[test]
fn standalone_verify_reports_pack_size_mismatch() {
    let directory = tempfile::tempdir().unwrap();
    let (_, output_path) = build_verify_fixture(directory.path());
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut raw = fs::read(&idx_path).unwrap();
    raw[16 + 10..16 + 14].copy_from_slice(&1u32.to_le_bytes());
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
