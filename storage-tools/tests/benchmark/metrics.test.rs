use poker_hands_storage_tools::benchmark::metrics::{
    build_totals, measure_benchmark_case, percentile, BenchmarkCaseResult,
};

#[test]
fn percentile_uses_linear_interpolation() {
    let values = vec![1.0, 2.0, 3.0, 4.0];

    assert_eq!(percentile(&[], 95.0), 0.0);
    assert_eq!(percentile(&values, 50.0), 2.5);
    assert_eq!(percentile(&values, 95.0), 3.8499999999999996);
    assert_eq!(percentile(&values, 99.0), 3.9699999999999998);
}

#[test]
fn build_totals_sums_cases() {
    let cases = vec![case("a", 2, 10.0, 1, 5), case("b", 3, 20.0, 2, 7)];

    let totals = build_totals(&cases);

    assert_eq!(totals.iterations, 5);
    assert_eq!(totals.total_ms, 30.0);
    assert!((totals.avg_qps - 166.66666666666669).abs() < 0.000001);
    assert_eq!(totals.error_count, 3);
    assert_eq!(totals.result_count, 12);
}

#[test]
fn measure_benchmark_case_tracks_warmup_results_and_errors() {
    let items = vec![1, 2, 3];
    let case = measure_benchmark_case("case", "desc", &items, 2, |item, _| {
        if *item == 2 {
            Err("boom".to_owned())
        } else {
            Ok(*item)
        }
    });

    assert_eq!(case.iterations, 3);
    assert_eq!(case.warmup_iterations, 2);
    assert_eq!(case.result_count, 4);
    assert_eq!(case.error_count, 1);
    assert_eq!(case.first_error.as_deref(), Some("boom"));
}

fn case(
    name: &str,
    iterations: usize,
    total_ms: f64,
    error_count: u64,
    result_count: u64,
) -> BenchmarkCaseResult {
    BenchmarkCaseResult {
        name: name.to_owned(),
        description: String::new(),
        iterations,
        warmup_iterations: 0,
        total_ms,
        avg_ms: 0.0,
        p50_ms: 0.0,
        p95_ms: 0.0,
        p99_ms: 0.0,
        max_ms: 0.0,
        qps: 0.0,
        result_count,
        error_count,
        first_error: None,
    }
}
