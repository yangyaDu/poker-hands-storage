use poker_hands_storage_service::domain::dimension::DimensionRef;
use poker_hands_storage_service::query::QueryService;
use poker_hands_storage_service::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_service::storage::sqlite::Connection;

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

    build_store(&BuildOptions {
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

    let service = QueryService::open(&output_path, 1, true).unwrap();
    let dimension = DimensionRef::with_default_strategy(6, 100);
    let result = service.query(&dimension, 1, "AsAh").unwrap();
    assert_eq!(result.hand_code, "AA");
    assert_eq!(result.actions.len(), 2);
    assert_eq!(
        service
            .get_concrete_lines(&dimension, "F-F-F")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        service
            .get_drill_scenario_lines("default", "UTG", 6, 100)
            .unwrap(),
        vec!["F-F-F"]
    );
}
