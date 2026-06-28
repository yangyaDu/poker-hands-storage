use poker_hands_storage_tools::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use range_store_core::dimension::{get_bin_file_name, get_idx_file_name};
use range_store_core::manifest::{load_manifest, queryable_dimensions};
use range_store_core::sqlite::Connection;
use range_store_core::DimensionReader;

#[test]
fn parses_dimension_name() {
    assert_eq!(
        DimensionSpec::parse("default:6:100").unwrap(),
        DimensionSpec {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
        }
    );
    assert!(DimensionSpec::parse("default:6").is_err());
}

#[test]
fn builds_queryable_store_from_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let source_path = dir.path().join("source.db");
    let output_path = dir.path().join("output");
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
               VALUES (1, 'F-F-F', 'F-F-F');
             INSERT INTO drill_scenario_lines_default
               VALUES (1, 'UTG', 'F-F-F', 6, 100);
             INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES
               (1, 'AA', 'fold', 0, 0, 0.25, NULL),
               (1, 'AA', 'raise', 2.5, 2.5, 0.75, 1.0);",
        )
        .unwrap();
    drop(source);

    let summary = build_store(&BuildOptions {
        source_db: source_path,
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
    assert_eq!(summary.dimensions.len(), 1);
    assert_eq!(summary.dimensions[0].pack_count, 1);

    // Verify manifest is valid
    let manifest = load_manifest(&output_path.join("manifest.json")).unwrap();
    let queryable = queryable_dimensions(&manifest).unwrap();
    assert_eq!(queryable.len(), 1);
    assert_eq!(queryable[0].strategy, "default");

    // Verify binary files are readable via DimensionReader
    let idx_path = output_path.join(get_idx_file_name("default", 6, 100));
    let bin_path = output_path.join(get_bin_file_name("default", 6, 100));
    let reader = DimensionReader::open(&idx_path, &bin_path).unwrap();
    // concrete_line_id = 1, hand_id for AA = 0, verify_checksum = true
    let result = reader.query(1, 0, true).unwrap().unwrap();
    assert_eq!(result.cells.len(), 2);
}
