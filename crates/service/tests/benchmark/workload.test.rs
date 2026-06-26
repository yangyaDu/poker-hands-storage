#[path = "../support/verify_store_fixture.rs"]
mod verify_store_fixture;

use poker_hands_storage_service::benchmark::benchmark_models::{
    BenchmarkWorkload, WorkloadMode, WorkloadOptions,
};
use poker_hands_storage_service::benchmark::workload::{
    create_benchmark_workload, parse_range_table_dimension, read_workload_json, write_workload_json,
};
use poker_hands_storage_service::domain::dimension::DimensionRef;
use tempfile::tempdir;
use verify_store_fixture::build_verify_fixture;

#[test]
fn parse_range_table_dimension_accepts_strategy_with_underscores() {
    let dimension = parse_range_table_dimension("range_data_special_strategy_8max_40BB").unwrap();

    assert_eq!(dimension.strategy, "special_strategy");
    assert_eq!(dimension.player_count, 8);
    assert_eq!(dimension.depth_bb, 40);
}

#[test]
fn create_workload_is_deterministic_for_seed() {
    let directory = tempdir().unwrap();
    let (source_path, _) = build_verify_fixture(directory.path());

    let options = WorkloadOptions {
        source_db_path: source_path,
        requested_dimensions: vec![DimensionRef::with_default_strategy(6, 100)],
        seed: 7,
        hand_iterations: 5,
        batch_iterations: 3,
        batch_size: 2,
        batch_sizes: vec![1, 2],
        workload_mode: WorkloadMode::Random,
    };

    let first = create_benchmark_workload(&options).unwrap();
    let second = create_benchmark_workload(&options).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.hand_queries.len(), 5);
    assert_eq!(first.batch_queries.len(), 3);
    assert_eq!(
        first
            .batch_queries_by_size
            .iter()
            .map(|(size, _)| *size)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn abstract_local_batches_use_same_dimension_and_requested_size() {
    let directory = tempdir().unwrap();
    let (source_path, _) = build_verify_fixture(directory.path());

    let workload = create_benchmark_workload(&WorkloadOptions {
        source_db_path: source_path,
        requested_dimensions: Vec::new(),
        seed: 3,
        hand_iterations: 2,
        batch_iterations: 2,
        batch_size: 2,
        batch_sizes: vec![2],
        workload_mode: WorkloadMode::AbstractLocal,
    })
    .unwrap();

    assert_eq!(workload.mode, WorkloadMode::AbstractLocal);
    for batch in workload.batch_queries {
        assert_eq!(batch.strategy, "default");
        assert_eq!(batch.player_count, 6);
        assert_eq!(batch.depth_bb, 100);
        assert_eq!(batch.requests.len(), 2);
    }
}

#[test]
fn workload_json_round_trip_and_legacy_fallback() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("workload.json");
    let legacy_path = directory.path().join("legacy-workload.json");
    let workload = BenchmarkWorkload {
        seed: 1,
        mode: WorkloadMode::Random,
        dimensions: vec!["default:6max:100BB".to_owned()],
        hand_queries: Vec::new(),
        batch_queries: Vec::new(),
        batch_size: 5,
        batch_queries_by_size: vec![(5, Vec::new())],
    };

    write_workload_json(&path, &workload).unwrap();
    assert_eq!(read_workload_json(&path).unwrap(), workload);

    std::fs::write(
        &legacy_path,
        r#"{
          "seed": 1,
          "mode": "random",
          "dimensions": ["default:6max:100BB"],
          "handQueries": [],
          "batchQueries": [{
            "strategy": "default",
            "playerCount": 6,
            "depthBb": 100,
            "requests": [{"concreteLineId": 1, "holeCards": "AA"}]
          }],
          "batchSize": 9
        }"#,
    )
    .unwrap();
    let loaded = read_workload_json(&legacy_path).unwrap();
    assert_eq!(loaded.batch_queries_by_size.len(), 1);
    assert_eq!(loaded.batch_queries_by_size[0].0, 9);
    assert_eq!(loaded.batch_queries.len(), 1);
}
