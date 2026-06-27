use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::benchmark::memory_snapshot::MemorySnapshot;
use crate::errors::AppError;

use super::cache_eviction::evict_cache;
use super::runner::{build_dimension_report, discover_dimensions, select_dimension_queries};
use super::types::{
    AggregateReport, BenchmarkSqliteColdCommand, ColdStartBenchmarkReport, ColdStartPhaseSummaries,
    ColdStartRunFailure, ColdStartRunResult, ColdWorkerOutput, ColdWorkerTimings,
    DimensionColdStartReport, DimensionQuery, LatencySummary, PhaseAccounting,
};

pub fn run_sqlite_cold_benchmark(
    command: &BenchmarkSqliteColdCommand,
) -> Result<ColdStartBenchmarkReport, AppError> {
    let dimensions = discover_dimensions(&command.dir, &command.requested_dimensions)?;
    if dimensions.is_empty() {
        return Err(AppError::invalid_argument(
            "No successful dimensions were found for SQLite cold-start benchmark.",
        ));
    }

    let dataset_size_bytes = source_db_size(&command.source);
    let filler_size_bytes = command.cache_filler_mb * 1024 * 1024;
    let queries = select_dimension_queries(
        &command.source,
        &dimensions,
        command.query_policy,
        command.fixed_concrete_line_id,
        command.fixed_hand.as_deref(),
    )?;

    let worker_binary = std::env::current_exe().map_err(|e| {
        AppError::new(
            "CURRENT_EXE",
            format!("Cannot determine executable path: {e}"),
        )
    })?;

    let mut dimension_reports = Vec::new();
    for query in &queries {
        let mut results = Vec::new();
        for run_index in 0..command.runs_per_dimension {
            let eviction = evict_cache(command.mode, filler_size_bytes, dataset_size_bytes);
            let run_result = run_worker(&worker_binary, command, query, run_index, eviction);
            let is_ok = run_result.ok;
            results.push(run_result);

            if command.fail_fast && !is_ok {
                break;
            }
            let errors = results.iter().filter(|r| !r.ok).count();
            if errors >= command.max_errors_per_dimension {
                break;
            }
        }
        dimension_reports.push(build_dimension_report(query, &results));
    }

    let report = build_report(command, &dimension_reports, filler_size_bytes);
    write_report(command, &report)?;
    Ok(report)
}

fn run_worker(
    worker_binary: &Path,
    command: &BenchmarkSqliteColdCommand,
    query: &DimensionQuery,
    run_index: usize,
    eviction: super::types::EvictionResult,
) -> ColdStartRunResult {
    let start = Instant::now();

    let mut cmd = Command::new(worker_binary);
    cmd.arg("sqlite-cold-worker")
        .args(["--source", &command.source.to_string_lossy()])
        .args(["--strategy", &query.strategy])
        .args(["--player-count", &query.player_count.to_string()])
        .args(["--depth-bb", &query.depth_bb.to_string()])
        .args(["--concrete-line-id", &query.concrete_line_id.to_string()])
        .args(["--hand", &query.hand]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return failed_run(
                run_index,
                eviction,
                -1,
                &format!("Failed to spawn SQLite worker: {e}"),
            );
        }
    };

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(e) => {
            return failed_run(
                run_index,
                eviction,
                -1,
                &format!("Failed to wait for SQLite worker: {e}"),
            );
        }
    };

    let process_elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout_text = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    let (worker_output, valid_json) = match serde_json::from_str::<ColdWorkerOutput>(&stdout_text) {
        Ok(parsed) => (parsed, true),
        Err(_) => {
            let stderr_text = String::from_utf8_lossy(&output.stderr);
            let error = format!(
                "SQLite worker did not return valid JSON. exitCode={exit_code}, stdout={}, stderr={}",
                &stdout_text[..stdout_text.len().min(500)],
                &stderr_text[..stderr_text.len().min(500)]
            );
            return failed_run(run_index, eviction, exit_code, &error);
        }
    };

    let combined_ok = worker_output.ok && exit_code == 0 && valid_json;
    let process_overhead_ms = (process_elapsed_ms - worker_output.timings.worker_total_ms).max(0.0);
    let phase_accounting = PhaseAccounting::compute(&worker_output.timings);

    ColdStartRunResult {
        ok: combined_ok,
        run_index,
        store_open_and_first_query_ms: worker_output.store_open_and_first_query_ms,
        result_count: worker_output.result_count,
        process_elapsed_ms,
        process_overhead_ms,
        memory_before: worker_output.memory_before,
        memory_after: worker_output.memory_after,
        timings: worker_output.timings,
        eviction,
        exit_code,
        valid_json,
        phase_accounting,
        error: worker_output.error,
    }
}

fn failed_run(
    run_index: usize,
    eviction: super::types::EvictionResult,
    exit_code: i32,
    error: &str,
) -> ColdStartRunResult {
    let empty_timings = ColdWorkerTimings {
        service_open_ms: 0.0,
        dimension_prewarm_ms: 0.0,
        first_query_ms: 0.0,
        close_ms: 0.0,
        worker_total_ms: 0.0,
    };
    ColdStartRunResult {
        ok: false,
        run_index,
        store_open_and_first_query_ms: 0.0,
        result_count: 0,
        process_elapsed_ms: 0.0,
        process_overhead_ms: 0.0,
        memory_before: empty_memory(),
        memory_after: empty_memory(),
        timings: empty_timings.clone(),
        eviction,
        exit_code,
        valid_json: false,
        phase_accounting: PhaseAccounting::compute(&empty_timings),
        error: Some(error.to_owned()),
    }
}

fn build_report(
    command: &BenchmarkSqliteColdCommand,
    dimension_reports: &[DimensionColdStartReport],
    filler_size_bytes: u64,
) -> ColdStartBenchmarkReport {
    let all_store_query_latencies: Vec<f64> = dimension_reports
        .iter()
        .filter(|d| d.success_count > 0)
        .map(|d| d.store_open_and_first_query_ms.avg_ms)
        .collect();

    let all_process_latencies: Vec<f64> = dimension_reports
        .iter()
        .filter(|d| d.success_count > 0)
        .map(|d| d.process_elapsed_ms.avg_ms)
        .collect();

    let all_failures: Vec<ColdStartRunFailure> = dimension_reports
        .iter()
        .flat_map(|d| d.failures.clone())
        .collect();

    let total_runs: usize = dimension_reports.iter().map(|d| d.runs).sum();
    let total_ok: usize = dimension_reports.iter().map(|d| d.success_count).sum();
    let total_errors: usize = dimension_reports.iter().map(|d| d.error_count).sum();

    let worst_accounting = dimension_reports
        .iter()
        .max_by(|a, b| {
            a.phase_accounting
                .unaccounted_ms
                .abs()
                .total_cmp(&b.phase_accounting.unaccounted_ms.abs())
        })
        .map(|d| d.phase_accounting.clone())
        .unwrap_or(PhaseAccounting {
            phase_sum_ms: 0.0,
            worker_total_ms: 0.0,
            unaccounted_ms: 0.0,
            unaccounted_ratio: 0.0,
        });

    let aggregate_phase = ColdStartPhaseSummaries {
        service_open_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.service_open_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
        dimension_prewarm_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.dimension_prewarm_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
        first_query_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.first_query_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
        close_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.close_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
        worker_total_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.worker_total_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
        process_overhead_ms: LatencySummary::from_values(
            &dimension_reports
                .iter()
                .filter(|d| d.success_count > 0)
                .map(|d| d.phase_timings.process_overhead_ms.avg_ms)
                .collect::<Vec<_>>(),
        ),
    };

    ColdStartBenchmarkReport {
        generated_at: crate::benchmark::report::generated_at_utc(),
        engine: "sqlite".to_owned(),
        mode: command.mode.to_string(),
        platform: std::env::consts::OS.to_owned(),
        runs_per_dimension: command.runs_per_dimension,
        source_db_path: command.source.to_string_lossy().to_string(),
        binary_dir: "not-applicable".to_owned(),
        meta_db_path: "not-applicable".to_owned(),
        verify_checksums: false,
        cache_filler_size_bytes: filler_size_bytes,
        dimensions: dimension_reports.to_vec(),
        aggregate: AggregateReport {
            dimensions: dimension_reports.len(),
            runs: total_runs,
            successful_runs: total_ok,
            error_count: total_errors,
            store_open_and_first_query_ms: LatencySummary::from_values(&all_store_query_latencies),
            process_elapsed_ms: LatencySummary::from_values(&all_process_latencies),
            phase_timings: aggregate_phase,
            phase_accounting: worst_accounting,
            failures: all_failures,
        },
        notes: build_notes(command, filler_size_bytes),
    }
}

fn build_notes(command: &BenchmarkSqliteColdCommand, filler_size_bytes: u64) -> Vec<String> {
    let mut notes = vec![
        "Each run starts a fresh Rust process and records SQLite open/query timings plus parent-observed process elapsed time.".to_owned(),
        "storeOpenAndFirstQueryMs = SQLite connection open + first query. dimensionPrewarmMs is zero because SQLite has no per-dimension mmap prewarm phase.".to_owned(),
        "First query time includes SQLite statement preparation, row scan for the selected hand, and action-row counting.".to_owned(),
        "Parent process overhead = parent-observed process elapsed time - worker-measured total; approximates Rust binary startup/shutdown and IPC overhead.".to_owned(),
        "Phase accounting records the difference between sum of individual phase timings and workerTotalMs. A discrepancy >1ms or ratio >1% should be investigated.".to_owned(),
        format!("Query policy: {:?}.", command.query_policy),
    ];

    match command.mode {
        super::types::ColdStartMode::ProcessCold => {
            notes.push("process-cold does not attempt OS page cache eviction; it measures fresh process/open/query cost with whatever cache state the OS currently has.".to_owned());
        }
        super::types::ColdStartMode::OsBestEffort => {
            notes.push(format!(
                "os-best-effort writes and reads a {:.1} MB non-zero filler file to perturb OS page cache. This is best-effort, not a guaranteed cold cache.",
                filler_size_bytes as f64 / (1024.0 * 1024.0)
            ));
        }
        super::types::ColdStartMode::LinuxDropCache => {
            notes.push("linux-drop-cache attempts sync + /proc/sys/vm/drop_caches and requires Linux with sufficient privileges.".to_owned());
        }
    }

    notes
}

fn write_report(
    command: &BenchmarkSqliteColdCommand,
    report: &ColdStartBenchmarkReport,
) -> Result<(), AppError> {
    super::report::write_cold_start_json(&command.out_path, report)?;
    super::report::write_cold_start_markdown(&command.md_path, report)?;
    Ok(())
}

fn source_db_size(path: &Path) -> u64 {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn empty_memory() -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: None,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: None,
    }
}
