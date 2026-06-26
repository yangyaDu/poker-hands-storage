use std::path::{Path, PathBuf};

use poker_hands_storage_service::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_service::storage::sqlite::Connection;

pub fn build_verify_fixture(root: &Path) -> (PathBuf, PathBuf) {
    let source_path = root.join("source.db");
    let output_path = root.join("output");
    let source = Connection::open(&source_path, false).unwrap();
    source
        .exec(
            "CREATE TABLE range_data_default_6max_100BB (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL NULL
             );
             CREATE TABLE concrete_lines_default_6max_100BB (
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE drill_scenario_lines_default (
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             INSERT INTO concrete_lines_default_6max_100BB
               VALUES (1, 'R-C', 'R2-C'), (2, 'R-C', 'R3-C');
             INSERT INTO drill_scenario_lines_default
               VALUES (1, 'UTG', 'R-C', 6, 0);
             INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES
               (1, 'AA', 'fold', 0, 0, 0.25, NULL),
               (1, 'AA', 'raise', 2.5, 2.5, 0.75, 1.0),
               (2, 'AKs', 'raise', 40, 2, 0.5, 5.0);",
        )
        .unwrap();
    drop(source);

    build_store(&BuildOptions {
        source_db: source_path.clone(),
        out_dir: output_path.clone(),
        dimensions: vec![DimensionSpec {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
        }],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
    })
    .unwrap();

    (source_path, output_path)
}
