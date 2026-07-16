use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};

use poker_hands_storage_tools::proto_range_storage::v3::format::{
    decode_payload_locator, decode_section_descriptor, ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
    ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME, DRILL_SCENARIOS_DATA_FILE_NAME,
    DRILL_SCENARIOS_INDEX_FILE_NAME, HEADER_SIZE, SECTION_DESCRIPTOR_SIZE,
};
use poker_hands_storage_tools::proto_range_storage::v3::metadata_store::{
    export_metadata, MetadataExportOptions, MetadataStore,
};
use range_store_core::dimension::DimensionSpec;
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::sqlite::Connection;

#[test]
fn metadata_export_filters_dimension_reassigns_ids_and_serves_queries() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "depth", false);
    let out_dir = temp.path().join("v3");
    let summary = export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 32,
        overwrite: false,
    })
    .unwrap();

    assert_eq!(summary.drill_scenarios.drill_count, 1);
    assert_eq!(summary.abstract_action_paths.abstract_path_count, 2);
    assert_eq!(summary.abstract_action_paths.concrete_path_count, 3);
    assert_eq!(
        summary
            .concrete_paths
            .iter()
            .map(|path| (path.source_id, path.concrete_action_path_id))
            .collect::<Vec<_>>(),
        vec![(10, 1), (30, 2), (50, 3)]
    );
    for file_name in [
        DRILL_SCENARIOS_DATA_FILE_NAME,
        DRILL_SCENARIOS_INDEX_FILE_NAME,
        ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
        ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
    ] {
        assert!(out_dir.join(file_name).is_file());
    }
    assert!(!out_dir.join("lines.db").exists());

    let store = MetadataStore::open(&out_dir).unwrap();
    assert_eq!(
        store.get_drill_scenario_lines("rfi").unwrap(),
        vec!["A".to_owned(), "B".to_owned()]
    );
    assert_eq!(
        store
            .get_drill_scenario_lines("other-dimension")
            .unwrap_err()
            .code(),
        "DRILL_SCENARIO_NOT_FOUND"
    );
    let abstract_rows = store
        .get_concrete_lines(ConcreteLineFilter::Abstract("A"))
        .unwrap();
    assert_eq!(abstract_rows.len(), 2);
    assert_eq!(abstract_rows[0].concrete_line_id, 1);
    assert_eq!(abstract_rows[1].concrete_line_id, 2);
    assert_eq!(store.resolve_concrete_action_path("B-1").unwrap(), 3);
    let concrete_row = store
        .get_concrete_lines(ConcreteLineFilter::Concrete("B-1"))
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(concrete_row.abstract_line, "B");
    assert_eq!(concrete_row.concrete_line_id, 3);
    assert_eq!(
        store
            .get_concrete_lines(ConcreteLineFilter::AbstractAndConcrete {
                abstract_line: "A",
                concrete_line: "B-1",
            })
            .unwrap_err()
            .code(),
        "CONCRETE_LINE_NOT_FOUND"
    );
}

#[test]
fn metadata_export_accepts_drill_depth_source_column() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "drill_depth", false);
    let out_dir = temp.path().join("v3");
    export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    assert_eq!(
        MetadataStore::open(out_dir)
            .unwrap()
            .get_drill_scenario_lines("rfi")
            .unwrap(),
        vec!["A".to_owned(), "B".to_owned()]
    );
}

#[test]
fn metadata_export_rejects_duplicate_concrete_action_paths() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "depth", true);
    let error = export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: temp.path().join("v3"),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap_err();
    assert_eq!(error.code(), "DUPLICATE_CONCRETE_ACTION_PATH");
}

#[test]
fn metadata_export_rejects_dangling_drill_abstract_path() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "depth", false);
    Connection::open(&source_db, false)
        .unwrap()
        .exec(
            "INSERT INTO drill_scenario_lines_default
             VALUES (4, 'broken', 'missing', 6, 100)",
        )
        .unwrap();
    let error = export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: temp.path().join("v3"),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap_err();
    assert_eq!(error.code(), "V3_DRILL_ABSTRACT_PATH_NOT_FOUND");
}

#[test]
fn metadata_reader_rejects_corrupt_proto_page() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "depth", false);
    let out_dir = temp.path().join("v3");
    export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();

    let index_bytes = fs::read(out_dir.join(DRILL_SCENARIOS_INDEX_FILE_NAME)).unwrap();
    let page_section =
        decode_section_descriptor(&index_bytes[HEADER_SIZE..HEADER_SIZE + SECTION_DESCRIPTOR_SIZE])
            .unwrap();
    let locator_start = page_section.offset as usize;
    let locator = decode_payload_locator(&index_bytes[locator_start..]).unwrap();
    let mut data = OpenOptions::new()
        .write(true)
        .open(out_dir.join(DRILL_SCENARIOS_DATA_FILE_NAME))
        .unwrap();
    data.seek(SeekFrom::Start(locator.offset)).unwrap();
    data.write_all(&[0xff]).unwrap();
    data.flush().unwrap();

    let error = MetadataStore::open(out_dir)
        .unwrap()
        .get_drill_scenario_lines("rfi")
        .unwrap_err();
    assert_eq!(error.code(), "INVALID_V3_METADATA");
}

#[test]
fn metadata_reader_rejects_truncated_index_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db, "depth", false);
    let out_dir = temp.path().join("v3");
    export_metadata(&MetadataExportOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    let index_path = out_dir.join(ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME);
    let bytes = fs::read(&index_path).unwrap();
    fs::write(&index_path, &bytes[..HEADER_SIZE + 1]).unwrap();

    let error = match MetadataStore::open(out_dir) {
        Ok(_) => panic!("truncated V3 index unexpectedly opened"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "INVALID_V3_METADATA");
}

fn dimension() -> DimensionSpec {
    DimensionSpec {
        strategy: "default".to_owned(),
        player_count: 6,
        depth_bb: 100,
    }
}

fn build_source_fixture(path: &std::path::Path, depth_column: &str, duplicate: bool) {
    let connection = Connection::open(path, false).unwrap();
    connection
        .exec(&format!(
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
               {depth_column} INTEGER NOT NULL
             );
             INSERT INTO concrete_lines_default_6max_100BB VALUES
               (10, 'A', 'A-1'),
               (30, 'A', 'A-2'),
               (50, 'B', 'B-1');
             INSERT INTO drill_scenario_lines_default VALUES
               (1, 'rfi', 'A', 6, 100),
               (2, 'rfi', 'B', 6, 100),
               (3, 'other-dimension', 'X', 9, 200);"
        ))
        .unwrap();
    if duplicate {
        connection
            .exec(
                "INSERT INTO concrete_lines_default_6max_100BB
                 VALUES (70, 'B', 'B-1')",
            )
            .unwrap();
    }
}
