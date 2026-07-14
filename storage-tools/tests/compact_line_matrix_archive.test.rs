use std::fs;
use std::process::Command;
use std::sync::Arc;

use poker_hands_storage_tools::proto_range_storage::line_matrix_store::{
    export_all_compact_line_matrix_archives, export_compact_line_matrix_archive,
    CompactLineMatrixArchive, CompactLineMatrixArchiveOptions, CompactLineMatrixArchivesOptions,
};
use poker_hands_storage_tools::proto_range_storage::proto::ActionType as CompactActionType;
use poker_hands_storage_tools::proto_range_storage::query_facade::ProtoRangeStoreFacade;
use poker_hands_storage_tools::proto_range_storage::query_service::ProtoRangeQueryService;
use poker_hands_storage_tools::range_store_builder::{build_store, BuildOptions};
use range_store_core::dimension::{DimensionRef, DimensionSpec};
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::query::{
    parse_action_filters, ActionFilter, ActionResult, FrequencyFilter, QueryBatchResult,
    QueryResult, StoreQueryService,
};
use range_store_core::sqlite::{Connection, Value};

#[test]
fn compact_archive_filters_null_ev_and_uses_action_local_compact_indexes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("compact-archive");

    let summary = export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export compact archive");

    assert_eq!(summary.matrix_count, 2);
    assert_eq!(summary.action_value_count, 504);
    assert_eq!(
        fs::metadata(&summary.index_path)
            .expect("index metadata")
            .len(),
        48
    );

    let archive = CompactLineMatrixArchive::open(&out_dir).expect("open compact archive");
    assert_eq!(archive.dimension().strategy, "default");
    assert_eq!(archive.dimension().player_count, 6);
    assert_eq!(archive.dimension().depth_bb, 100);
    let verification = archive.verify_all().expect("verify compact archive");
    let sequential_verification = archive
        .verify_all_sequential()
        .expect("verify compact archive sequentially");
    assert_eq!(verification, sequential_verification);
    assert_eq!(verification.matrix_count, 2);
    assert_eq!(verification.action_count, 5);
    assert_eq!(verification.action_value_count, 504);
    let first = archive.read_matrix(1).expect("read first compact matrix");
    let second = archive.read_matrix(1).expect("read cached compact matrix");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(archive.matrix_cache_stats().hits, 1);
    assert_eq!(archive.matrix_cache_stats().misses, 1);
    let matrix = first.matrix();
    assert_eq!(matrix.schema_version, 2);
    assert_eq!(matrix.valid_hand_bitmap.len(), 22);
    assert_eq!(matrix.valid_hand_bitmap[0], 0xff);
    assert!(!bit_is_set(&matrix.valid_hand_bitmap, 168));

    let call_index = matrix
        .actions
        .iter()
        .position(|action| action.action_type == CompactActionType::Call as i32)
        .expect("call action");
    let call = &matrix.actions[call_index];
    assert_eq!(call.action_hand_bitmap.len(), 21);
    assert_eq!(call.frequency_x10000.len(), 167);
    assert_eq!(call.ev_x10000.len(), 167);

    assert_eq!(first.action_value(call_index, 1), None);
    assert_eq!(first.action_value(call_index, 168), None);
    assert_eq!(
        first.action_value(call_index, 2),
        Some(
            poker_hands_storage_tools::proto_range_storage::line_matrix_store::HandActionValue {
                frequency_x10000: 2_500,
                ev_x10000: 0,
            }
        )
    );
    assert_eq!(
        first.action_value_by_identity(CompactActionType::Call, 0, 0, 2),
        first.action_value(call_index, 2)
    );
    assert_eq!(
        first.action_value_by_identity(CompactActionType::Allin, 0, 0, 2),
        None
    );
}

#[test]
fn compact_archive_does_not_scan_null_ev_rows() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "UPDATE range_data_default_6max_100BB
             SET action_name = 'unsupported', frequency = 2.0
             WHERE concrete_line_id = 1 AND hole_cards = 'AKs'
               AND action_name = 'call'",
        )
        .expect("make ignored NULL EV row invalid in other fields");

    let summary = export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: temp.path().join("compact-archive"),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("NULL EV rows must not be scanned by Proto");

    assert_eq!(summary.matrix_count, 2);
}

#[test]
fn proto_query_service_returns_core_query_shape_without_null_ev_actions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("compact-archive");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export compact archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open query service");
    let dimension = DimensionRef::with_default_strategy(6, 100);
    let aa: QueryResult = service
        .query_hand_strategy(&dimension, 1, "AA")
        .expect("query AA");

    assert_eq!(aa.actions.len(), 2);
    assert_eq!(aa.actions[0].action_name, "fold");
    assert_eq!(aa.actions[0].action_size, 0.0);
    assert_eq!(aa.actions[0].amount_bb, 0.0);
    assert_eq!(aa.actions[0].frequency, 0.5);
    assert_eq!(aa.actions[0].hand_ev, Some(-0.25));
    assert_eq!(aa.actions[1].action_name, "call");
    assert_eq!(aa.actions[1].frequency, 0.5);
    assert_eq!(aa.actions[1].hand_ev, Some(0.5));

    let aks = service
        .query_hand_strategy(&dimension, 1, "AKs")
        .expect("query AKs");
    let action_names = aks
        .actions
        .iter()
        .map(|action| action.action_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(action_names, ["fold", "raise"]);
}

#[test]
fn proto_query_service_matches_core_after_null_ev_filtering() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let proto_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db: source_db.clone(),
        out_dir: proto_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db,
        out_dir: core_dir.clone(),
        dimensions: vec![DimensionSpec::parse("default:6:100").expect("dimension")],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");

    let dimension = DimensionRef::with_default_strategy(6, 100);
    let core = StoreQueryService::open(&core_dir, 2, true).expect("open core query service");
    let proto = ProtoRangeQueryService::open(&proto_dir).expect("open Proto query service");

    for (concrete_line_id, hand) in [(1, "AA"), (1, "AKs"), (1, "AQs"), (2, "AA")] {
        let core_result = core
            .query(&dimension, concrete_line_id, hand)
            .expect("core query");
        let proto_result = proto
            .query_hand_strategy(&dimension, concrete_line_id, hand)
            .expect("Proto query");
        let retained_core = core_result
            .actions
            .iter()
            .filter(|action| action.hand_ev.is_some())
            .collect::<Vec<_>>();

        assert_eq!(retained_core.len(), proto_result.actions.len());
        for (core_action, proto_action) in retained_core.into_iter().zip(&proto_result.actions) {
            assert_actions_match(core_action, proto_action);
        }
    }
}

#[test]
fn proto_query_service_uses_core_style_not_found_codes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open Proto query service");
    let dimension = DimensionRef::with_default_strategy(6, 100);

    let error = service
        .query_hand_strategy(&DimensionRef::with_default_strategy(9, 100), 1, "AA")
        .expect_err("wrong dimension must fail");
    assert_eq!(error.code(), "DIMENSION_NOT_FOUND");

    let error = service
        .query_hand_strategy(&dimension, 3, "AA")
        .expect_err("missing line must fail");
    assert_eq!(error.code(), "CONCRETE_LINE_NOT_FOUND");

    let error = service
        .query_hand_strategy(&dimension, 1, "22")
        .expect_err("hand without retained actions must fail");
    assert_eq!(error.code(), "HAND_STRATEGY_NOT_FOUND");
}

#[test]
fn proto_query_service_batch_preserves_request_order_for_repeated_lines() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open Proto query service");
    let dimension = DimensionRef::with_default_strategy(6, 100);
    let requests = vec![
        (1, "AKs".to_owned()),
        (2, "AA".to_owned()),
        (1, "AA".to_owned()),
    ];
    let batch: QueryBatchResult = service
        .query_batch(&dimension, &requests)
        .expect("query batch");

    assert_eq!(batch.results.len(), 3);
    assert_eq!(batch.results[0].concrete_line_id, 1);
    assert_eq!(batch.results[0].hole_cards, "AKs");
    assert_eq!(batch.results[1].concrete_line_id, 2);
    assert_eq!(batch.results[1].hole_cards, "AA");
    assert_eq!(batch.results[2].concrete_line_id, 1);
    assert_eq!(batch.results[2].hole_cards, "AA");
    assert_eq!(
        batch.results[0]
            .actions
            .iter()
            .map(|action| action.action_name.as_str())
            .collect::<Vec<_>>(),
        ["fold", "raise"]
    );
    assert_eq!(batch.results[1].actions.len(), 2);
    assert_eq!(batch.results[2].actions.len(), 2);
}

#[test]
fn proto_query_service_batch_reports_the_lowest_failing_request() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open Proto query service");
    let requests = vec![(3, "AA".to_owned()), (1, "not-a-hand".to_owned())];
    let error = service
        .query_batch(&DimensionRef::with_default_strategy(6, 100), &requests)
        .expect_err("batch must fail");

    assert_eq!(error.code(), "BATCH_ITEM_ERROR");
    assert!(error.message().contains("requests[0]"));
    assert!(error.message().contains("concrete_line_id=3"));
}

#[test]
fn proto_query_service_batch_matches_core_after_null_ev_filtering() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let proto_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db: source_db.clone(),
        out_dir: proto_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db,
        out_dir: core_dir.clone(),
        dimensions: vec![DimensionSpec::parse("default:6:100").expect("dimension")],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");

    let dimension = DimensionRef::with_default_strategy(6, 100);
    let requests = vec![
        (1, "AKs".to_owned()),
        (2, "AA".to_owned()),
        (1, "AA".to_owned()),
    ];
    let core = StoreQueryService::open(&core_dir, 2, true).expect("open core query service");
    let proto = ProtoRangeQueryService::open(&proto_dir).expect("open Proto query service");
    let core_batch = core
        .query_batch(&dimension, &requests)
        .expect("core batch query");
    let proto_batch = proto
        .query_batch(&dimension, &requests)
        .expect("Proto batch query");

    assert_eq!(core_batch.results.len(), proto_batch.results.len());
    for (core_item, proto_item) in core_batch.results.iter().zip(&proto_batch.results) {
        assert_eq!(core_item.concrete_line_id, proto_item.concrete_line_id);
        assert_eq!(core_item.hole_cards, proto_item.hole_cards);
        let retained_core = core_item
            .actions
            .iter()
            .filter(|action| action.hand_ev.is_some())
            .collect::<Vec<_>>();
        assert_eq!(retained_core.len(), proto_item.actions.len());
        for (core_action, proto_action) in retained_core.into_iter().zip(&proto_item.actions) {
            assert_actions_match(core_action, proto_action);
        }
    }
}

#[test]
fn proto_query_service_filters_hands_by_actions_with_core_semantics() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open Proto query service");
    let dimension = DimensionRef::with_default_strategy(6, 100);
    let call = parse_action_filters(vec!["call".to_owned()]).expect("call filter");
    let call_hands = service
        .query_hands_by_actions(&dimension, 1, &call, Some(0.2))
        .expect("filter call hands");
    assert_eq!(call_hands.len(), 167);
    assert!(call_hands.contains(&"AA".to_owned()));
    assert!(!call_hands.contains(&"AKs".to_owned()));
    assert!(call_hands.contains(&"AQs".to_owned()));
    assert!(!call_hands.contains(&"22".to_owned()));

    let raise = parse_action_filters(vec!["raise2".to_owned()]).expect("raise filter");
    let raise_hands = service
        .query_hands_by_actions(&dimension, 1, &raise, Some(0.2))
        .expect("filter raise hands");
    assert_eq!(raise_hands.len(), 167);
    assert!(!raise_hands.contains(&"AA".to_owned()));
    assert!(raise_hands.contains(&"AKs".to_owned()));

    let any =
        parse_action_filters(vec!["call".to_owned(), "raise2".to_owned()]).expect("OR filters");
    let any_hands = service
        .query_hands_by_actions(&dimension, 1, &any, Some(0.2))
        .expect("filter OR hands");
    assert_eq!(any_hands.len(), 168);
    assert!(any_hands.contains(&"AA".to_owned()));
    assert!(any_hands.contains(&"AKs".to_owned()));
}

#[test]
fn proto_hands_by_actions_matches_core_after_null_ev_filtering() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let proto_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db: source_db.clone(),
        out_dir: proto_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db,
        out_dir: core_dir.clone(),
        dimensions: vec![DimensionSpec::parse("default:6:100").expect("dimension")],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");

    let dimension = DimensionRef::with_default_strategy(6, 100);
    let core = StoreQueryService::open(&core_dir, 2, true).expect("open core query service");
    let proto = ProtoRangeQueryService::open(&proto_dir).expect("open Proto query service");
    for filters in [
        parse_action_filters(vec!["call".to_owned()]).expect("call filter"),
        parse_action_filters(vec!["raise2".to_owned()]).expect("raise filter"),
        parse_action_filters(vec!["call".to_owned(), "raise2".to_owned()]).expect("OR filters"),
    ] {
        let proto_hands = proto
            .query_hands_by_actions(&dimension, 1, &filters, Some(0.2))
            .expect("Proto hands by actions");
        let core_hands =
            core_hands_by_actions_without_null_ev(&core, &dimension, 1, &filters, Some(0.2));
        assert_eq!(proto_hands, core_hands);
    }
}

#[test]
fn proto_query_service_accepts_raw_action_filter_names() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("proto");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto archive");

    let service = ProtoRangeQueryService::open(&out_dir).expect("open Proto query service");
    let hands = service
        .query_hands_by_action_names(
            &DimensionRef::with_default_strategy(6, 100),
            1,
            &["call".to_owned(), "raise2".to_owned()],
            Some(0.2),
        )
        .expect("query hands by action names");

    assert_eq!(hands.len(), 168);
    assert!(hands.contains(&"AA".to_owned()));
    assert!(hands.contains(&"AKs".to_owned()));
}

#[test]
fn cli_exports_compact_default_6max_100bb_archive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("compact-archive");

    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "export-compact-line-matrix-archive",
            "--source-db",
            source_db.to_str().expect("source path"),
            "--out-dir",
            out_dir.to_str().expect("output path"),
        ])
        .output()
        .expect("run compact archive command");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_dir.join("manifest.json").is_file());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("Compact LineMatrix archive export complete."));
}

#[test]
fn exports_and_reports_all_discovered_compact_dimensions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB AS
               SELECT * FROM concrete_lines_default_6max_100BB;
             CREATE TABLE range_data_default_8max_200BB AS
               SELECT * FROM range_data_default_6max_100BB;",
        )
        .expect("clone fixture dimension");
    drop(connection);

    let out_dir = temp.path().join("all-compact-archives");
    let report = export_all_compact_line_matrix_archives(&CompactLineMatrixArchivesOptions {
        source_db: source_db.clone(),
        out_dir: out_dir.clone(),
        overwrite: false,
    })
    .expect("export all compact dimensions");

    assert_eq!(
        report.sqlite_bytes,
        fs::metadata(source_db).expect("sqlite metadata").len()
    );
    assert_eq!(report.dimensions.len(), 2);
    assert_eq!(report.dimensions[0].matrix_count, 2);
    assert_eq!(report.dimensions[1].matrix_count, 2);
    assert_eq!(report.dimensions[0].action_value_count, 504);
    assert_eq!(report.dimensions[1].action_value_count, 504);
    assert_eq!(
        report.total_bin_idx_bytes,
        report.total_data_bytes + report.total_index_bytes
    );
    assert!(report.bin_idx_to_sqlite_ratio > 0.0);
    assert!(out_dir.join("storage-comparison.json").is_file());
    assert!(out_dir
        .join("default_6max_100BB")
        .join("matrices.lmbin")
        .is_file());
    assert!(out_dir
        .join("default_8max_200BB")
        .join("matrices.lmidx")
        .is_file());
}

#[test]
fn proto_range_store_facade_discovers_dimensions_and_limits_open_handles() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB AS
               SELECT * FROM concrete_lines_default_6max_100BB;
             CREATE TABLE range_data_default_8max_200BB AS
               SELECT * FROM range_data_default_6max_100BB;",
        )
        .expect("clone fixture dimension");
    drop(connection);

    let root_dir = temp.path().join("all-compact-archives");
    export_all_compact_line_matrix_archives(&CompactLineMatrixArchivesOptions {
        source_db,
        out_dir: root_dir.clone(),
        overwrite: false,
    })
    .expect("export all compact dimensions");

    let facade = ProtoRangeStoreFacade::open(&root_dir, 1, true).expect("open Proto facade");
    assert_eq!(
        facade.known_dimensions(),
        vec![
            "default:6max:100BB".to_owned(),
            "default:8max:200BB".to_owned()
        ]
    );
    assert_eq!(facade.open_handle_count(), 0);

    let six_max = DimensionRef::with_default_strategy(6, 100);
    assert_eq!(
        facade
            .matrix_count(&six_max)
            .expect("read 6max matrix count"),
        2
    );
    assert_eq!(facade.open_handle_count(), 1);
    let six_max_result = facade
        .query_hand_strategy(&six_max, 1, "AA")
        .expect("query 6max hand");
    assert_eq!(six_max_result.actions.len(), 2);
    assert_eq!(facade.open_handle_count(), 1);

    let eight_max = DimensionRef::with_default_strategy(8, 200);
    let eight_max_result = facade
        .query_hand_strategy(&eight_max, 1, "AA")
        .expect("query 8max hand");
    assert_eq!(eight_max_result.actions.len(), 2);
    assert_eq!(facade.open_handle_count(), 1);

    facade.prewarm(&six_max).expect("prewarm 6max handle");
    assert_eq!(facade.open_handle_count(), 1);

    let error = facade
        .query_hand_strategy(&DimensionRef::with_default_strategy(9, 100), 1, "AA")
        .expect_err("unknown dimension must fail");
    assert_eq!(error.code(), "DIMENSION_NOT_FOUND");
}

#[test]
fn proto_range_store_facade_reads_per_dimension_metadata() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open source metadata");
    connection
        .exec(
            "CREATE TABLE drill_scenario_lines_default(
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             INSERT INTO drill_scenario_lines_default(
               drill_name, abstract_line, player_count, depth
             ) VALUES ('rfi', 'F-F-F', 6, 100);",
        )
        .expect("create source drill metadata");
    drop(connection);
    let proto_root = temp.path().join("proto-root");
    let archive_dir = proto_root.join("default_6max_100BB");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: archive_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export compact archive");
    let facade = ProtoRangeStoreFacade::open(&proto_root, 1, true).expect("open Proto facade");
    let dimension = DimensionRef::with_default_strategy(6, 100);
    let abstract_lines = facade
        .get_concrete_lines(&dimension, ConcreteLineFilter::Abstract("F-F-F"))
        .expect("find abstract lines");
    assert_eq!(abstract_lines.len(), 1);
    assert_eq!(abstract_lines[0].concrete_line_id, 1);
    assert_eq!(abstract_lines[0].concrete_line, "F-F-F");
    assert_eq!(facade.open_handle_count(), 1);

    let concrete_lines = facade
        .get_concrete_lines(&dimension, ConcreteLineFilter::Concrete("R2-F"))
        .expect("find concrete lines");
    assert_eq!(concrete_lines.len(), 1);
    assert_eq!(concrete_lines[0].concrete_line_id, 2);
    let drill_lines = facade
        .get_drill_scenario_lines("default", "rfi", 6, 100)
        .expect("find drill lines");
    assert_eq!(drill_lines, ["F-F-F"]);
    let connection = Connection::open(&archive_dir.join("lines.db"), false)
        .expect("open exported drill metadata");
    connection
        .exec(
            "INSERT INTO drill_scenario_lines_default(
               drill_name, abstract_line, player_count, drill_depth
             ) VALUES ('rfi', 'R-F', 6, 100);",
        )
        .expect("mutate exported drill metadata");
    drop(connection);
    assert_eq!(
        facade
            .get_drill_scenario_lines("default", "rfi", 6, 100)
            .expect("read cached drill lines"),
        ["F-F-F"]
    );
}

#[test]
fn compact_vs_core_benchmark_uses_the_proto_storage_root() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB AS
               SELECT * FROM concrete_lines_default_6max_100BB;
             CREATE TABLE range_data_default_8max_200BB AS
               SELECT * FROM range_data_default_6max_100BB;",
        )
        .expect("clone fixture dimension");
    drop(connection);

    let proto_root = temp.path().join("proto-root");
    export_all_compact_line_matrix_archives(&CompactLineMatrixArchivesOptions {
        source_db: source_db.clone(),
        out_dir: proto_root.clone(),
        overwrite: false,
    })
    .expect("export all compact dimensions");

    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db,
        out_dir: core_dir.clone(),
        dimensions: vec![
            DimensionSpec::parse("default:6:100").expect("6max dimension"),
            DimensionSpec::parse("default:8:200").expect("8max dimension"),
        ],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");

    let report_path = temp.path().join("benchmark.json");
    let markdown_path = temp.path().join("benchmark.md");
    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "benchmark-compact-vs-core",
            "--compact-dir",
            proto_root.to_str().expect("Proto root"),
            "--core-dir",
            core_dir.to_str().expect("core directory"),
            "--dimension",
            "default:6:100",
            "--hot-iterations",
            "2",
            "--warmup-iterations",
            "0",
            "--cold-runs",
            "1",
            "--concrete-line-id",
            "1",
            "--hand-id",
            "0",
            "--out",
            report_path.to_str().expect("report path"),
            "--md",
            markdown_path.to_str().expect("markdown path"),
        ])
        .output()
        .expect("run compact/core benchmark");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read benchmark report"))
            .expect("parse benchmark report");
    assert_eq!(
        report["compactStorageRoot"],
        proto_root.to_str().expect("Proto root")
    );
    assert_eq!(report["matrixCount"], 2);
    assert!(markdown_path.is_file());
}

#[test]
fn three_way_hot_benchmark_reports_shared_proto_v2_strategy_cases() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB AS
               SELECT * FROM concrete_lines_default_6max_100BB;
             CREATE TABLE range_data_default_8max_200BB AS
               SELECT * FROM range_data_default_6max_100BB;
             CREATE TABLE drill_scenario_lines_default(
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             INSERT INTO drill_scenario_lines_default(
               drill_name, abstract_line, player_count, depth
             ) VALUES ('rfi', 'F-F-F', 6, 100);",
        )
        .expect("clone fixture dimension");
    drop(connection);

    let proto_root = temp.path().join("proto-root");
    export_all_compact_line_matrix_archives(&CompactLineMatrixArchivesOptions {
        source_db: source_db.clone(),
        out_dir: proto_root.clone(),
        overwrite: false,
    })
    .expect("export all compact dimensions");
    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db: source_db.clone(),
        out_dir: core_dir.clone(),
        dimensions: vec![
            DimensionSpec::parse("default:6:100").expect("6max dimension"),
            DimensionSpec::parse("default:8:200").expect("8max dimension"),
        ],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");

    let report_path = temp.path().join("three-way.json");
    let markdown_path = temp.path().join("three-way.md");
    let workload_path = temp.path().join("three-way-workload.json");
    fs::write(
        &workload_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "seed": 1,
            "mode": "random",
            "dimensions": ["default:6max:100BB"],
            "handQueries": [
                {"strategy": "default", "playerCount": 6, "depthBb": 100, "concreteLineId": 1, "holeCards": "AA"}
            ],
            "batchQueries": [
                {"strategy": "default", "playerCount": 6, "depthBb": 100, "requests": [
                    {"concreteLineId": 1, "holeCards": "AA"},
                    {"concreteLineId": 1, "holeCards": "AQs"}
                ]}
            ],
            "batchSize": 2,
            "batchQueriesBySize": [
                [1, [{"strategy": "default", "playerCount": 6, "depthBb": 100, "requests": [
                    {"concreteLineId": 1, "holeCards": "AA"}
                ]}]],
                [2, [{"strategy": "default", "playerCount": 6, "depthBb": 100, "requests": [
                    {"concreteLineId": 1, "holeCards": "AA"},
                    {"concreteLineId": 1, "holeCards": "AQs"}
                ]}]]
            ],
            "handsByActionsQueries": [
                {"strategy": "default", "playerCount": 6, "depthBb": 100, "concreteLineId": 1, "actions": ["call"], "frequency": 0.2}
            ],
            "drillScenarioQueries": [
                {"strategy": "default", "drillName": "rfi", "playerCount": 6, "drillDepth": 100}
            ]
        }))
        .expect("serialize workload"),
    )
    .expect("write workload");
    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "benchmark-three-way-hot",
            "--source",
            source_db.to_str().expect("source path"),
            "--proto-root",
            proto_root.to_str().expect("Proto root"),
            "--core-dir",
            core_dir.to_str().expect("core directory"),
            "--dimension",
            "default:6:100",
            "--workload",
            workload_path.to_str().expect("workload path"),
            "--warmup-iterations",
            "0",
            "--out",
            report_path.to_str().expect("report path"),
            "--md",
            markdown_path.to_str().expect("markdown path"),
        ])
        .output()
        .expect("run three-way hot benchmark");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read three-way report"))
            .expect("parse three-way report");
    assert_eq!(report["semanticProfile"], "proto-v2-non-null-ev");
    assert_eq!(report["cases"].as_array().expect("cases").len(), 7);
    assert!(report["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .all(|case| {
            case["resultCountMatch"].as_bool() == Some(true)
                && case["core"]["errorCount"].as_u64() == Some(0)
                && case["proto"]["errorCount"].as_u64() == Some(0)
                && case["sqlite"]["errorCount"].as_u64() == Some(0)
        }));
    assert!(report["excludedCases"]
        .as_array()
        .expect("excluded cases")
        .is_empty());
    assert!(report["memory"]["core"]["total"]["after"].is_object());
    assert!(report["memory"]["proto"]["total"]["after"].is_object());
    assert!(report["memory"]["sqlite"]["total"]["after"].is_object());
    assert!(markdown_path.is_file());

    let stability_report_path = temp.path().join("three-way-stability.json");
    let stability_markdown_path = temp.path().join("three-way-stability.md");
    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "benchmark-three-way-stability",
            "--runs",
            "2",
            "--source",
            source_db.to_str().expect("source path"),
            "--proto-root",
            proto_root.to_str().expect("Proto root"),
            "--core-dir",
            core_dir.to_str().expect("core directory"),
            "--dimension",
            "default:6:100",
            "--workload",
            workload_path.to_str().expect("workload path"),
            "--warmup-iterations",
            "0",
            "--out",
            stability_report_path
                .to_str()
                .expect("stability report path"),
            "--md",
            stability_markdown_path
                .to_str()
                .expect("stability markdown path"),
        ])
        .output()
        .expect("run three-way stability benchmark");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stability: serde_json::Value =
        serde_json::from_slice(&fs::read(&stability_report_path).expect("read stability report"))
            .expect("parse stability report");
    assert_eq!(stability["runs"], 2);
    assert_eq!(
        stability["cases"]
            .as_array()
            .expect("stability cases")
            .len(),
        7
    );
    assert_eq!(
        stability["metadataCache"]["postEvictionQueryMs"],
        serde_json::Value::Null
    );
    assert_eq!(stability["handStrategyProfile"]["samples"], 1);
    assert!(stability["handStrategyProfile"]["matrixReadMs"].is_object());
    assert_eq!(
        stability["handStrategyProfile"]["slowest"]
            .as_array()
            .expect("slowest")
            .len(),
        1
    );
    assert!(stability_markdown_path.is_file());
}

#[test]
fn three_way_cold_benchmark_reports_phase_and_memory_deltas() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open source drill metadata");
    connection
        .exec(
            "CREATE TABLE drill_scenario_lines_default(
               id INTEGER PRIMARY KEY,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               depth INTEGER NOT NULL
             );
             INSERT INTO drill_scenario_lines_default(
               drill_name, abstract_line, player_count, depth
             ) VALUES ('rfi', 'F-F-F', 6, 100);",
        )
        .expect("create source drill metadata");
    drop(connection);
    let proto_root = temp.path().join("proto-root");
    export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db: source_db.clone(),
        out_dir: proto_root.join("default_6max_100BB"),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export Proto storage");
    let core_dir = temp.path().join("core");
    build_store(&BuildOptions {
        source_db: source_db.clone(),
        out_dir: core_dir.clone(),
        dimensions: vec![DimensionSpec::parse("default:6:100").expect("dimension")],
        max_concrete_lines_per_dimension: None,
        overwrite: false,
        resume: false,
    })
    .expect("build core store");
    let report_path = temp.path().join("three-way-cold.json");
    let markdown_path = temp.path().join("three-way-cold.md");

    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "benchmark-three-way-cold",
            "--source",
            source_db.to_str().expect("source path"),
            "--proto-root",
            proto_root.to_str().expect("Proto root"),
            "--core-dir",
            core_dir.to_str().expect("core directory"),
            "--dimension",
            "default:6:100",
            "--operation",
            "drill-scenarios-metadata",
            "--drill-name",
            "rfi",
            "--drill-depth",
            "100",
            "--runs",
            "2",
            "--out",
            report_path.to_str().expect("report path"),
            "--md",
            markdown_path.to_str().expect("markdown path"),
        ])
        .output()
        .expect("run three-way cold benchmark");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read cold report"))
            .expect("parse cold report");
    assert_eq!(report["runsPerEngine"], 2);
    assert_eq!(report["operation"], "drill-scenarios-metadata");
    assert_eq!(report["query"], "rfi / 6max / 100BB");
    assert!(report["core"]["memory"]["totalRssBytes"].is_object());
    assert!(report["proto"]["memory"]["totalRssBytes"].is_object());
    assert!(report["sqlite"]["memory"]["totalRssBytes"].is_object());
    assert_eq!(report["core"]["errorCount"], 0);
    assert_eq!(report["proto"]["errorCount"], 0);
    assert_eq!(report["sqlite"]["errorCount"], 0);
    assert!(markdown_path.is_file());
}

fn create_source_fixture(path: &std::path::Path) {
    let connection = Connection::open(path, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_6max_100BB(
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE range_data_default_6max_100BB(
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL
             );",
        )
        .expect("create fixture tables");
    connection
        .execute(
            "INSERT INTO concrete_lines_default_6max_100BB(id, abstract_line, concrete_line)
             VALUES (?1, ?2, ?3)",
            &[
                Value::from(1u32),
                Value::from("F-F-F"),
                Value::from("F-F-F"),
            ],
        )
        .expect("insert line");
    connection
        .execute(
            "INSERT INTO concrete_lines_default_6max_100BB(id, abstract_line, concrete_line)
             VALUES (?1, ?2, ?3)",
            &[Value::from(2u32), Value::from("R-F"), Value::from("R2-F")],
        )
        .expect("insert second line");

    connection.exec("BEGIN").expect("begin fixture transaction");
    let mut insert = connection
        .prepare(
            "INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .expect("prepare range insert");
    for hand_idx in 0u8..=168 {
        // 22 has no rows at all on this line; this is distinct from an invalid hand.
        if hand_idx == 168 {
            continue;
        }
        let hand = hand_code_from_id(hand_idx);
        let fold_frequency = if hand_idx == 0 {
            0.5
        } else if hand_idx == 167 {
            0.24
        } else {
            0.25
        };
        let call_frequency = if hand_idx == 0 {
            0.5
        } else if hand_idx == 167 {
            0.24
        } else {
            0.25
        };
        insert_row(
            &mut insert,
            &hand,
            "fold",
            0.0,
            0.0,
            fold_frequency,
            Value::from(-0.25f64),
        );
        let call_ev = if hand_idx == 1 {
            Value::Null
        } else if hand_idx == 2 {
            Value::from(0.0f64)
        } else {
            Value::from(0.5f64)
        };
        insert_row(
            &mut insert,
            &hand,
            "call",
            0.0,
            0.0,
            call_frequency,
            call_ev,
        );
        if hand_idx != 0 {
            insert_row(
                &mut insert,
                &hand,
                "raise",
                40.0,
                2.0,
                0.5,
                Value::from(1.25f64),
            );
        }
    }
    insert_row_for_line(
        &mut insert,
        2,
        "AA",
        "fold",
        0.0,
        0.0,
        0.25,
        Value::from(-0.5f64),
    );
    insert_row_for_line(
        &mut insert,
        2,
        "AA",
        "raise",
        40.0,
        2.0,
        0.75,
        Value::from(1.5f64),
    );
    drop(insert);
    connection
        .exec("COMMIT")
        .expect("commit fixture transaction");
}

#[allow(clippy::too_many_arguments)]
fn insert_row(
    insert: &mut range_store_core::sqlite::Statement<'_>,
    hand: &str,
    action: &str,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Value,
) {
    insert_row_for_line(
        insert,
        1,
        hand,
        action,
        action_size,
        amount_bb,
        frequency,
        hand_ev,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_row_for_line(
    insert: &mut range_store_core::sqlite::Statement<'_>,
    concrete_line_id: u32,
    hand: &str,
    action: &str,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Value,
) {
    insert
        .execute(&[
            Value::from(concrete_line_id),
            Value::from(hand),
            Value::from(action),
            Value::from(action_size),
            Value::from(amount_bb),
            Value::from(frequency),
            hand_ev,
        ])
        .expect("insert range row");
}

fn bit_is_set(bitmap: &[u8], hand_idx: usize) -> bool {
    bitmap[hand_idx / 8] & (1u8 << (hand_idx % 8)) != 0
}

fn assert_actions_match(core: &ActionResult, proto: &ActionResult) {
    assert_eq!(core.action_name, proto.action_name);
    assert_eq!(core.action_size, proto.action_size);
    assert_eq!(core.amount_bb, proto.amount_bb);
    assert_eq!(core.frequency, proto.frequency);
    assert_eq!(core.hand_ev, proto.hand_ev);
}

fn core_hands_by_actions_without_null_ev(
    service: &StoreQueryService,
    dimension: &DimensionRef,
    concrete_line_id: u32,
    filters: &[ActionFilter],
    frequency: Option<f64>,
) -> Vec<String> {
    let frequency_filter = FrequencyFilter::from_request(frequency);
    (0u8..=168)
        .filter_map(|hand_id| {
            let hand = hand_code_from_id(hand_id);
            let result = service.query(dimension, concrete_line_id, &hand).ok()?;
            let matches = result
                .actions
                .iter()
                .filter(|action| action.hand_ev.is_some())
                .any(|action| {
                    frequency_filter.matches(action.frequency)
                        && (filters.is_empty()
                            || filters.iter().any(|filter| {
                                action.action_name == filter.action_name.as_str()
                                    && match filter.amount_bb {
                                        Some(amount_bb) => {
                                            (action.amount_bb - amount_bb).abs() <= f32::EPSILON
                                        }
                                        None => true,
                                    }
                            }))
                });
            matches.then_some(hand)
        })
        .collect()
}
