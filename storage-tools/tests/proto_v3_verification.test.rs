use std::fs;
use std::path::Path;

use poker_hands_storage_tools::proto_range_storage::v3::archive::{
    export_v3_archive, verify_v3_archive_before_publish, V3ArchiveExportOptions,
};
use poker_hands_storage_tools::proto_range_storage::v3::format::{
    decode_payload_locator, encode_payload_locator, DRILL_SCENARIOS_DATA_FILE_NAME,
    DRILL_SCENARIOS_INDEX_FILE_NAME, HAND_STRATEGIES_DATA_FILE_NAME, HEADER_SIZE,
    SECTION_DESCRIPTOR_SIZE,
};
use poker_hands_storage_tools::proto_range_storage::v3::manifest::{read_manifest, write_manifest};
use poker_hands_storage_tools::proto_range_storage::v3::proto::DrillScenarioPage;
use poker_hands_storage_tools::proto_range_storage::v3::verification::{
    cross_verify_sqlite_v3, verify_v3_archive, V3VerificationOptions,
};
use prost::Message;
use range_store_core::crc32c::crc32c;
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::Connection;

#[test]
fn clean_archive_passes_standalone_and_full_sqlite_cross_verification() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    let archive_dir = temp.path().join("archive");
    build_source_fixture(&source_db);
    export_fixture(&source_db, &archive_dir);

    let standalone = verify_v3_archive(&archive_dir, options());
    assert!(standalone.ok, "{:?}", standalone.failure_samples);
    assert_eq!(standalone.counts.files_checked, 6);
    assert_eq!(standalone.counts.concrete_action_paths, 3);
    assert_eq!(standalone.counts.hand_strategies, 3);
    verify_v3_archive_before_publish(&archive_dir).unwrap();

    let cross = cross_verify_sqlite_v3(&source_db, &archive_dir, options());
    assert!(cross.ok, "{:?}", cross.failure_samples);
    assert_eq!(cross.counts.hands_visited, 3 * 169);
    assert_eq!(cross.counts.action_cells_compared, 4 * 169);
    assert_eq!(cross.counts.source_action_cells, 5);
    assert_eq!(cross.counts.null_ev_cells, 1);
}

#[test]
fn cross_verifier_reports_mapping_cell_and_null_differences() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    let archive_dir = temp.path().join("archive");
    build_source_fixture(&source_db);
    export_fixture(&source_db, &archive_dir);

    Connection::open(&source_db, false)
        .unwrap()
        .exec(
            "UPDATE concrete_lines_default_6max_100BB
               SET concrete_line = 'A-X' WHERE id = 30;
             UPDATE drill_scenario_lines_default
               SET drill_name = 'other' WHERE abstract_line = 'B';
             UPDATE range_data_default_6max_100BB
               SET frequency = 0.6, hand_ev = 1.5
               WHERE concrete_line_id = 10 AND action_name = 'raise';
             UPDATE range_data_default_6max_100BB
               SET hand_ev = 0.0
               WHERE concrete_line_id = 10 AND action_name = 'fold';",
        )
        .unwrap();

    let report = cross_verify_sqlite_v3(&source_db, &archive_dir, options());
    assert!(!report.ok);
    assert!(report.counts.mapping_differences >= 2);
    assert!(report.counts.cell_differences >= 2);
    assert!(has_code(&report, "V3_DRILL_MAPPING_MISMATCH"));
    assert!(has_code(&report, "V3_CONCRETE_MAPPING_MISMATCH"));
    assert!(has_code(&report, "V3_ACTION_CELL_MISMATCH"));
    assert!(has_code(&report, "V3_NULL_EV_MISMATCH"));
}

#[test]
fn standalone_verifier_reports_whole_file_crc_damage() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    let archive_dir = temp.path().join("archive");
    build_source_fixture(&source_db);
    export_fixture(&source_db, &archive_dir);

    let path = archive_dir.join(HAND_STRATEGIES_DATA_FILE_NAME);
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    fs::write(path, bytes).unwrap();

    let report = verify_v3_archive(&archive_dir, options());
    assert!(!report.ok);
    assert!(has_code(&report, "INVALID_V3_MANIFEST"));
}

#[test]
fn standalone_verifier_reports_dangling_drill_reference_after_valid_crc_rewrite() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    let archive_dir = temp.path().join("archive");
    build_source_fixture(&source_db);
    export_fixture(&source_db, &archive_dir);
    rewrite_drill_reference_with_valid_checksums(&archive_dir);

    let report = verify_v3_archive(&archive_dir, options());
    assert!(!report.ok);
    assert!(has_code(&report, "INVALID_V3_METADATA"));
    assert!(report
        .failure_samples
        .iter()
        .any(|failure| failure.message.contains("missing abstract action path")));

    let error = verify_v3_archive_before_publish(&archive_dir).unwrap_err();
    assert_eq!(error.code(), "VERIFY_ERROR");
    assert!(error.message().contains("standalone verification failed"));
}

fn options() -> V3VerificationOptions {
    V3VerificationOptions {
        max_failure_samples: 100,
    }
}

fn has_code(
    report: &poker_hands_storage_tools::proto_range_storage::v3::verification::V3VerificationReport,
    code: &str,
) -> bool {
    report
        .failure_samples
        .iter()
        .any(|failure| failure.code == code)
}

fn export_fixture(source_db: &Path, archive_dir: &Path) {
    export_v3_archive(&V3ArchiveExportOptions {
        source_db: source_db.to_path_buf(),
        out_dir: archive_dir.to_path_buf(),
        dimension: DimensionSpec {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
        },
        metadata_page_target_bytes: 4096,
        overwrite: false,
    })
    .unwrap();
}

fn rewrite_drill_reference_with_valid_checksums(archive_dir: &Path) {
    let data_path = archive_dir.join(DRILL_SCENARIOS_DATA_FILE_NAME);
    let index_path = archive_dir.join(DRILL_SCENARIOS_INDEX_FILE_NAME);
    let mut data = fs::read(&data_path).unwrap();
    let mut index = fs::read(&index_path).unwrap();
    let locator_offset = HEADER_SIZE + 2 * SECTION_DESCRIPTOR_SIZE;
    let mut locator = decode_payload_locator(&index[locator_offset..locator_offset + 16]).unwrap();
    let start = locator.offset as usize;
    let end = start + locator.byte_length as usize;
    let mut page = DrillScenarioPage::decode(&data[start..end]).unwrap();
    let reference = page
        .entries
        .iter_mut()
        .flat_map(|entry| &mut entry.abstract_action_paths)
        .find(|path| path.as_str() == "A")
        .unwrap();
    *reference = "Z".to_owned();
    let encoded = page.encode_to_vec();
    assert_eq!(encoded.len(), locator.byte_length as usize);
    data[start..end].copy_from_slice(&encoded);
    locator.crc32c = crc32c(&encoded);
    index[locator_offset..locator_offset + 16].copy_from_slice(&encode_payload_locator(locator));
    fs::write(&data_path, &data).unwrap();
    fs::write(&index_path, &index).unwrap();

    let mut manifest = read_manifest(archive_dir).unwrap();
    manifest.drill_scenarios.data.crc32c = crc32c(&data);
    manifest.drill_scenarios.index.crc32c = crc32c(&index);
    write_manifest(archive_dir, &manifest).unwrap();
}

fn build_source_fixture(path: &Path) {
    Connection::open(path, false)
        .unwrap()
        .exec(
            "CREATE TABLE concrete_lines_default_6max_100BB(
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE drill_scenario_lines_default(
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             CREATE TABLE range_data_default_6max_100BB(
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL
             );
             INSERT INTO concrete_lines_default_6max_100BB VALUES
               (10, 'A', 'A-1'),
               (30, 'A', 'A-2'),
               (50, 'B', 'B-1');
             INSERT INTO drill_scenario_lines_default VALUES
               (1, 'rfi', 'A', 6, 100),
               (2, 'rfi', 'B', 6, 100);
             INSERT INTO range_data_default_6max_100BB VALUES
               (10, 'AA', 'fold', 0.0, 0.0, 0.0, NULL),
               (10, 'AA', 'raise', 2.5, 2.5, 0.5, 1.25),
               (10, 'KK', 'fold', 0.0, 0.0, 1.0, -0.5),
               (30, 'AKs', 'call', 1.0, 1.0, 1.0, 0.0),
               (50, 'QQ', 'check', 0.0, 0.0, 1.0, 0.75);",
        )
        .unwrap();
}
