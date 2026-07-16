use std::fs;
use std::path::{Path, PathBuf};

use poker_hands_storage_tools::proto_range_storage::v3::archive::{
    export_v3_archive, V3ArchiveExportOptions,
};
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::Connection;

/// Build the service fixture in the production V3 directory shape. The SQLite file exists only as
/// exporter input and is not placed under the returned runtime root.
pub fn build_api_test_store(root: &Path) -> PathBuf {
    let source_db = root.join("source.db");
    Connection::open(&source_db, false)
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
               drill_depth INTEGER NOT NULL
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
               (1, 'F-F-F', 'F-F-F');
             INSERT INTO drill_scenario_lines_default VALUES
               (1, 'rfi', 'F-F-F', 6, 100);
             INSERT INTO range_data_default_6max_100BB VALUES
               (1, 'AA', 'fold', 0.0, 0.0, 0.25, -0.1),
               (1, 'AA', 'raise', 2.5, 2.5, 0.75, 1.0),
               (1, 'KK', 'raise', 2.5, 2.5, 0.6, 0.8),
               (1, '72o', 'fold', 0.0, 0.0, 0.0, NULL);",
        )
        .unwrap();

    let output = root.join("output");
    fs::create_dir(&output).unwrap();
    export_v3_archive(&V3ArchiveExportOptions {
        source_db,
        out_dir: output.join("default_6max_100BB"),
        dimension: DimensionSpec {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
        },
        metadata_page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    output
}

pub fn build_empty_store(root: &Path) -> PathBuf {
    let output = root.join("output-empty");
    fs::create_dir(&output).unwrap();
    output
}
