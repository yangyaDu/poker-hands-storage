use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkCaseResult {
    pub name: String,
    pub description: String,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub total_ms: f64,
    pub avg_ms: f64,
    pub p50_ms: f64,
    #[serde(default)]
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub qps: f64,
    pub result_count: u64,
    pub error_count: u64,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTotals {
    pub iterations: usize,
    pub total_ms: f64,
    pub avg_qps: f64,
    pub error_count: u64,
    pub result_count: u64,
}

pub fn measure_benchmark_case<T, F>(
    name: &str,
    description: &str,
    items: &[T],
    warmup_iterations: usize,
    mut operation: F,
) -> BenchmarkCaseResult
where
    F: FnMut(&T, usize) -> Result<usize, String>,
{
    let effective_warmup = items.len().min(warmup_iterations);
    for (index, item) in items.iter().take(effective_warmup).enumerate() {
        let _ = operation(item, index);
    }

    let mut result_count = 0_u64;
    let mut error_count = 0_u64;
    let mut first_error = None;
    let mut times_ms = Vec::with_capacity(items.len());

    let case_start = Instant::now();
    for (index, item) in items.iter().enumerate() {
        let iteration_start = Instant::now();
        match operation(item, index) {
            Ok(count) => result_count += count as u64,
            Err(error) => {
                error_count += 1;
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        times_ms.push(iteration_start.elapsed().as_secs_f64() * 1000.0);
    }
    let total_ms = case_start.elapsed().as_secs_f64() * 1000.0;
    let avg_ms = safe_ratio(total_ms, items.len() as f64);

    times_ms.sort_by(|left, right| left.total_cmp(right));
    BenchmarkCaseResult {
        name: name.to_owned(),
        description: description.to_owned(),
        iterations: items.len(),
        warmup_iterations: effective_warmup,
        total_ms,
        avg_ms,
        p50_ms: percentile(&times_ms, 50.0),
        p90_ms: percentile(&times_ms, 90.0),
        p95_ms: percentile(&times_ms, 95.0),
        p99_ms: percentile(&times_ms, 99.0),
        max_ms: times_ms.last().copied().unwrap_or_default(),
        qps: safe_ratio(items.len() as f64, total_ms / 1000.0),
        result_count,
        error_count,
        first_error,
    }
}

pub fn build_totals(cases: &[BenchmarkCaseResult]) -> BenchmarkTotals {
    let iterations = cases.iter().map(|case| case.iterations).sum::<usize>();
    let total_ms = cases.iter().map(|case| case.total_ms).sum::<f64>();
    let error_count = cases.iter().map(|case| case.error_count).sum::<u64>();
    let result_count = cases.iter().map(|case| case.result_count).sum::<u64>();

    BenchmarkTotals {
        iterations,
        total_ms,
        avg_qps: safe_ratio(iterations as f64, total_ms / 1000.0),
        error_count,
        result_count,
    }
}

pub fn percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = (percentile / 100.0) * (sorted.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    if lower == upper {
        return sorted[lower];
    }
    let fraction = index - lower as f64;
    sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
}

pub fn safe_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator == 0.0 || !denominator.is_finite() {
        0.0
    } else {
        numerator / denominator
    }
}
