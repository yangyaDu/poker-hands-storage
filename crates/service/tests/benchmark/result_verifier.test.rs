#[path = "../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use poker_hands_storage_service::benchmark::hot::result_verifier::verify_benchmark_results;
use poker_hands_storage_service::benchmark::types::HandBenchmarkItem;
use poker_hands_storage_service::query::QueryService;
use poker_hands_storage_service::storage::sqlite::Connection;
use tempfile::tempdir;
use verify_store_fixture::build_verify_fixture;

#[test]
fn result_verifier_matches_source_and_binary_action_counts() {
    let directory = tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let service = QueryService::open(&output_path, 3, true).unwrap();

    let summary = verify_benchmark_results(
        &source_path,
        &service,
        &[HandBenchmarkItem {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
            concrete_line_id: 1,
            hole_cards: "AA".to_owned(),
        }],
    )
    .unwrap();

    assert_eq!(summary.sample_size, 1);
    assert_eq!(summary.match_count, 1);
    assert_eq!(summary.mismatch_count, 0);
    assert_eq!(summary.error_count, 0);
    assert!(!summary.has_errors());
}

#[test]
fn result_verifier_reports_action_count_mismatch() {
    let directory = tempdir().unwrap();
    let (source_path, output_path) = build_verify_fixture(directory.path());
    let source = Connection::open(&source_path, false).unwrap();
    source
        .exec(
            "INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES (1, 'AA', 'call', 1, 1, 0.1, NULL);",
        )
        .unwrap();
    drop(source);

    let service = QueryService::open(&output_path, 3, true).unwrap();
    let summary = verify_benchmark_results(
        &source_path,
        &service,
        &[HandBenchmarkItem {
            strategy: "default".to_owned(),
            player_count: 6,
            depth_bb: 100,
            concrete_line_id: 1,
            hole_cards: "AA".to_owned(),
        }],
    )
    .unwrap();

    assert_eq!(summary.match_count, 0);
    assert_eq!(summary.mismatch_count, 1);
    assert_eq!(summary.error_count, 0);
    assert!(summary.mismatches[0].contains("SQLite=3"));
    assert!(summary.has_errors());
}
