use std::fs;
use std::sync::Arc;

use poker_hands_storage_tools::proto_range_storage::v3::archive::{
    export_v3_archive, V3Archive, V3ArchiveExportOptions, V3ArchiveOpenOptions,
};
use poker_hands_storage_tools::proto_range_storage::v3::facade::{V3Facade, V3FacadeOptions};
use poker_hands_storage_tools::proto_range_storage::v3::format::{
    ABSTRACT_ACTION_PATHS_DATA_FILE_NAME, ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
    DRILL_SCENARIOS_DATA_FILE_NAME, DRILL_SCENARIOS_INDEX_FILE_NAME,
    HAND_STRATEGIES_DATA_FILE_NAME, HAND_STRATEGIES_INDEX_FILE_NAME,
};
use poker_hands_storage_tools::proto_range_storage::v3::manifest::MANIFEST_FILE_NAME;
use poker_hands_storage_tools::proto_range_storage::v3::query_service::V3QueryService;
use range_store_core::dimension::{DimensionRef, DimensionSpec};
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::sqlite::Connection;

#[test]
fn archive_query_service_serves_full_business_chain_and_uses_bounded_caches() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    let archive_dir = root.join("default_6max_100BB");
    let summary = export_v3_archive(&V3ArchiveExportOptions {
        source_db,
        out_dir: archive_dir.clone(),
        dimension: dimension_spec(),
        metadata_page_target_bytes: 32,
        overwrite: false,
    })
    .unwrap();
    assert_eq!(summary.manifest.hand_strategies.record_count, 3);
    for file_name in [
        MANIFEST_FILE_NAME,
        DRILL_SCENARIOS_DATA_FILE_NAME,
        DRILL_SCENARIOS_INDEX_FILE_NAME,
        ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
        ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
        HAND_STRATEGIES_DATA_FILE_NAME,
        HAND_STRATEGIES_INDEX_FILE_NAME,
    ] {
        assert!(archive_dir.join(file_name).is_file());
    }
    assert!(!archive_dir.join("lines.db").exists());

    let service = V3QueryService::open_with_options(
        &archive_dir,
        V3ArchiveOpenOptions {
            verify_file_checksums: true,
            metadata_cache_byte_budget: 16 * 1024,
            strategy_cache_byte_budget: 16 * 1024,
        },
    )
    .unwrap();
    let dimension = dimension_ref();
    let first = service.query_hand_strategy(&dimension, 1, "AA").unwrap();
    let fold = first
        .actions
        .iter()
        .find(|action| action.action_name == "fold")
        .unwrap();
    assert_eq!(fold.frequency, 0.0);
    assert_eq!(fold.hand_ev, None);
    let raise = first
        .actions
        .iter()
        .find(|action| action.action_name == "raise")
        .unwrap();
    assert_eq!(raise.frequency, 0.5);
    assert_eq!(raise.hand_ev, Some(1.25));
    service.query_hand_strategy(&dimension, 1, "AA").unwrap();
    let strategy_stats = service.strategy_cache_stats();
    assert_eq!(strategy_stats.misses, 1);
    assert_eq!(strategy_stats.hits, 1);
    assert!(strategy_stats.resident_estimated_bytes <= 16 * 1024);

    let by_path = service
        .query_hand_strategy_by_path(&dimension, "B-1", "QQ")
        .unwrap();
    assert_eq!(by_path.actions[0].hand_ev, Some(0.75));
    assert_eq!(
        service.get_drill_scenario_lines(&dimension, "rfi").unwrap(),
        vec!["A".to_owned(), "B".to_owned()]
    );
    service.get_drill_scenario_lines(&dimension, "rfi").unwrap();
    let rows = service
        .get_concrete_lines(&dimension, ConcreteLineFilter::Abstract("A"))
        .unwrap();
    assert_eq!(rows.len(), 2);
    let metadata_stats = service.metadata_cache_stats();
    assert!(metadata_stats.hits >= 1);
    assert!(metadata_stats.resident_estimated_bytes <= 16 * 1024);

    let facade = V3Facade::open_with_options(
        &root,
        V3FacadeOptions {
            max_open_handles: 1,
            verify_file_checksums: true,
            metadata_cache_byte_budget_per_handle: 16 * 1024,
            strategy_cache_byte_budget_per_handle: 16 * 1024,
        },
    )
    .unwrap();
    assert_eq!(facade.known_dimensions(), vec!["default:6max:100BB"]);
    facade
        .query_hand_strategy_by_path(&dimension, "A-1", "AA")
        .unwrap();
    facade
        .query_hand_strategy_by_path(&dimension, "A-1", "AA")
        .unwrap();
    assert_eq!(facade.handle_pool_stats().opens, 1);
    assert!(facade.handle_pool_stats().hits >= 1);
    assert_eq!(facade.cache_stats().open_handle_count, 1);
}

#[test]
fn zero_byte_budgets_disable_both_runtime_caches() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    let archive_dir = temp.path().join("archive");
    export_v3_archive(&V3ArchiveExportOptions {
        source_db,
        out_dir: archive_dir.clone(),
        dimension: dimension_spec(),
        metadata_page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    let service = V3QueryService::open_with_options(
        archive_dir,
        V3ArchiveOpenOptions {
            verify_file_checksums: false,
            metadata_cache_byte_budget: 0,
            strategy_cache_byte_budget: 0,
        },
    )
    .unwrap();
    service
        .query_hand_strategy_by_path(&dimension_ref(), "A-1", "AA")
        .unwrap();
    assert_eq!(service.metadata_cache_stats().resident_estimated_bytes, 0);
    assert!(service.metadata_cache_stats().cache_disabled_skips >= 1);
    assert_eq!(service.strategy_cache_stats().resident_estimated_bytes, 0);
    assert_eq!(service.strategy_cache_stats().cache_disabled_skips, 1);
}

#[test]
fn archive_open_with_full_checksums_rejects_modified_file() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    let archive_dir = temp.path().join("archive");
    export_v3_archive(&V3ArchiveExportOptions {
        source_db,
        out_dir: archive_dir.clone(),
        dimension: dimension_spec(),
        metadata_page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    let data_path = archive_dir.join(DRILL_SCENARIOS_DATA_FILE_NAME);
    let mut bytes = fs::read(&data_path).unwrap();
    let last = bytes.last_mut().unwrap();
    *last ^= 0xff;
    fs::write(data_path, bytes).unwrap();
    let error = match V3Archive::open_with_options(
        archive_dir,
        V3ArchiveOpenOptions {
            verify_file_checksums: true,
            ..V3ArchiveOpenOptions::default()
        },
    ) {
        Ok(_) => panic!("modified V3 archive unexpectedly opened"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "INVALID_V3_MANIFEST");
}

#[test]
fn facade_evicts_dimension_handles_and_supports_concurrent_queries() {
    let temp = tempfile::tempdir().unwrap();
    let source_db = temp.path().join("source.db");
    build_source_fixture(&source_db);
    add_second_dimension_fixture(&source_db);
    let root = temp.path().join("root");
    fs::create_dir(&root).unwrap();
    export_v3_archive(&V3ArchiveExportOptions {
        source_db: source_db.clone(),
        out_dir: root.join("default_6max_100BB"),
        dimension: dimension_spec(),
        metadata_page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();
    export_v3_archive(&V3ArchiveExportOptions {
        source_db,
        out_dir: root.join("default_8max_200BB"),
        dimension: second_dimension_spec(),
        metadata_page_target_bytes: 1024,
        overwrite: false,
    })
    .unwrap();

    let facade = Arc::new(
        V3Facade::open_with_options(
            root,
            V3FacadeOptions {
                max_open_handles: 1,
                verify_file_checksums: false,
                metadata_cache_byte_budget_per_handle: 8 * 1024,
                strategy_cache_byte_budget_per_handle: 8 * 1024,
            },
        )
        .unwrap(),
    );
    facade
        .query_hand_strategy(&dimension_ref(), 1, "AA")
        .unwrap();
    facade
        .query_hand_strategy(&second_dimension_ref(), 1, "JJ")
        .unwrap();
    facade
        .query_hand_strategy(&dimension_ref(), 1, "AA")
        .unwrap();
    assert_eq!(facade.handle_pool_stats().opens, 3);
    assert_eq!(facade.handle_pool_stats().evictions, 2);
    assert_eq!(facade.cache_stats().open_handle_count, 1);

    let threads = (0..4)
        .map(|_| {
            let facade = Arc::clone(&facade);
            std::thread::spawn(move || {
                facade
                    .query_hand_strategy(&dimension_ref(), 1, "AA")
                    .unwrap()
                    .actions
                    .len()
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 2);
    }
}

fn dimension_spec() -> DimensionSpec {
    DimensionSpec {
        strategy: "default".to_owned(),
        player_count: 6,
        depth_bb: 100,
    }
}

fn dimension_ref() -> DimensionRef {
    DimensionRef::new("default", 6, 100)
}

fn second_dimension_spec() -> DimensionSpec {
    DimensionSpec {
        strategy: "default".to_owned(),
        player_count: 8,
        depth_bb: 200,
    }
}

fn second_dimension_ref() -> DimensionRef {
    DimensionRef::new("default", 8, 200)
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

fn add_second_dimension_fixture(path: &std::path::Path) {
    Connection::open(path, false)
        .unwrap()
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB(
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE range_data_default_8max_200BB(
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL
             );
             INSERT INTO concrete_lines_default_8max_200BB VALUES (5, 'C', 'C-1');
             INSERT INTO drill_scenario_lines_default VALUES (3, 'three-bet', 'C', 8, 200);
             INSERT INTO range_data_default_8max_200BB VALUES
               (5, 'JJ', 'raise', 3.0, 3.0, 1.0, 2.0);",
        )
        .unwrap();
}
