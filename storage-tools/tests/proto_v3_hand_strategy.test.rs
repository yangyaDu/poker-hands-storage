use poker_hands_storage_tools::proto_range_storage::v3::metadata_store::{
    export_metadata, MetadataExportOptions,
};
use poker_hands_storage_tools::proto_range_storage::v3::proto::ActionType;
use poker_hands_storage_tools::proto_range_storage::v3::strategy_codec::{
    validate_hand_strategy, NULL_EV_FREQUENCY_SENTINEL,
};
use poker_hands_storage_tools::proto_range_storage::v3::strategy_store::{
    export_hand_strategies, HandStrategyExportOptions, HandStrategyStore,
};
use range_store_core::dimension::DimensionSpec;
use range_store_core::hole_cards::get_hand_id;
use range_store_core::sqlite::Connection;

#[test]
fn hand_strategy_export_preserves_null_ev_and_quantized_values() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    let out_dir = temp.path().join("v3");
    let metadata = export_metadata(&MetadataExportOptions {
        source_db: source_db.clone(),
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    let manifest = export_hand_strategies(
        &HandStrategyExportOptions {
            source_db,
            out_dir: out_dir.clone(),
            dimension: dimension(),
            overwrite: false,
        },
        &metadata.concrete_paths,
    )
    .unwrap();
    assert_eq!(manifest.record_count, 3);

    let store = HandStrategyStore::open(&out_dir).unwrap();
    assert_eq!(store.record_count(), 3);
    let strategy = store.read(1).unwrap();
    let aa = usize::from(get_hand_id("AA").unwrap());
    let kk = usize::from(get_hand_id("KK").unwrap());
    let fold_index = strategy
        .strategy()
        .actions
        .iter()
        .position(|action| action.action_type == ActionType::Fold as i32)
        .unwrap();
    let raise_index = strategy
        .strategy()
        .actions
        .iter()
        .position(|action| action.action_type == ActionType::Raise as i32)
        .unwrap();

    let null_ev = strategy.action_value(fold_index, aa).unwrap();
    assert_eq!(null_ev.frequency_x10000, 0);
    assert_eq!(null_ev.hand_ev_x10000, 0);
    assert!(null_ev.hand_ev_is_null);
    let negative_ev = strategy.action_value(fold_index, kk).unwrap();
    assert_eq!(negative_ev.frequency_x10000, 10_000);
    assert_eq!(negative_ev.hand_ev_x10000, -5_000);
    assert!(!negative_ev.hand_ev_is_null);
    let raise = strategy.action_value(raise_index, aa).unwrap();
    assert_eq!(raise.frequency_x10000, 5_000);
    assert_eq!(raise.hand_ev_x10000, 12_500);
    assert!(!raise.hand_ev_is_null);
    assert_eq!(
        strategy.strategy().actions[fold_index].frequency_x10000[0],
        NULL_EV_FREQUENCY_SENTINEL
    );
    assert_eq!(store.read(0).unwrap_err().code(), "CONCRETE_LINE_NOT_FOUND");
    assert_eq!(store.read(4).unwrap_err().code(), "CONCRETE_LINE_NOT_FOUND");
}

#[test]
fn hand_strategy_export_rejects_null_ev_with_nonzero_frequency() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    Connection::open(&source_db, false)
        .unwrap()
        .exec(
            "UPDATE range_data_default_6max_100BB
             SET frequency = 0.25
             WHERE concrete_line_id = 10 AND hole_cards = 'AA' AND action_name = 'fold'",
        )
        .unwrap();
    let out_dir = temp.path().join("v3");
    let metadata = export_metadata(&MetadataExportOptions {
        source_db: source_db.clone(),
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    let error = export_hand_strategies(
        &HandStrategyExportOptions {
            source_db,
            out_dir,
            dimension: dimension(),
            overwrite: false,
        },
        &metadata.concrete_paths,
    )
    .unwrap_err();
    assert_eq!(error.code(), "NULL_EV_WITH_NONZERO_FREQUENCY");
}

#[test]
fn hand_strategy_validation_rejects_invalid_null_sentinel_pair() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    let out_dir = temp.path().join("v3");
    let metadata = export_metadata(&MetadataExportOptions {
        source_db: source_db.clone(),
        out_dir: out_dir.clone(),
        dimension: dimension(),
        page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    export_hand_strategies(
        &HandStrategyExportOptions {
            source_db,
            out_dir: out_dir.clone(),
            dimension: dimension(),
            overwrite: false,
        },
        &metadata.concrete_paths,
    )
    .unwrap();
    let decoded = HandStrategyStore::open(out_dir).unwrap().read(1).unwrap();
    let mut invalid = decoded.strategy().clone();
    let sentinel_index = invalid.actions[0]
        .frequency_x10000
        .iter()
        .position(|frequency| *frequency == NULL_EV_FREQUENCY_SENTINEL)
        .unwrap();
    invalid.actions[0].hand_ev_x10000[sentinel_index] = 1;
    assert_eq!(
        validate_hand_strategy(&invalid).unwrap_err().code(),
        "INVALID_V3_HAND_STRATEGY"
    );
}

fn dimension() -> DimensionSpec {
    DimensionSpec {
        strategy: "default".to_owned(),
        player_count: 6,
        depth_bb: 100,
    }
}

fn build_source_fixture(path: &std::path::Path) {
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
