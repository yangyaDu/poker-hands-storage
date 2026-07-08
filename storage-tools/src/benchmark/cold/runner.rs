use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::errors::ToolError;
use range_store_core::dimension::{dimension_key, DimensionRef};
use range_store_core::manifest::load_manifest;

use crate::benchmark::memory_snapshot::MemorySnapshot;
use crate::benchmark::report::{write_cold_start_json, write_cold_start_markdown};

use super::cache_eviction::{compute_dataset_size, evict_cache};
use super::types::{
    AggregateReport, BenchmarkColdCommand, ColdStartBenchmarkReport, ColdStartPhaseSummaries,
    ColdStartRunFailure, ColdStartRunResult, ColdWorkerOutput, ColdWorkerTimings,
    DimensionColdStartReport, DimensionQuery, LatencySummary, PhaseAccounting, QueryPolicy,
};

/// Run the complete cold-start benchmark.
pub fn run_cold_benchmark(
    command: &BenchmarkColdCommand,
) -> Result<ColdStartBenchmarkReport, ToolError> {
    let dimensions = discover_dimensions(&command.dir, &command.requested_dimensions)?;
    if dimensions.is_empty() {
        return Err(ToolError::invalid_argument(
            "No successful Range Strata Binary dimensions were found for cold-start benchmark.",
        ));
    }

    let dataset_size_bytes = compute_dataset_size(&command.dir);
    let filler_size_bytes = command.cache_filler_mb * 1024 * 1024;
    let queries = select_dimension_queries(
        &command.source,
        &dimensions,
        command.query_policy,
        command.fixed_concrete_line_id,
        command.fixed_hand.as_deref(),
    )?;

    let worker_binary = std::env::current_exe().map_err(|e| {
        ToolError::new(
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

    let report = build_report(
        command,
        &dimension_reports,
        dataset_size_bytes,
        filler_size_bytes,
    );
    write_report(command, &report)?;
    Ok(report)
}

pub(crate) fn discover_dimensions(
    dir: &Path,
    requested: &[DimensionRef],
) -> Result<Vec<DimensionInfo>, ToolError> {
    let manifest = load_manifest(&dir.join("manifest.json"))?;
    let mut dimensions: Vec<DimensionInfo> = manifest
        .dimensions
        .iter()
        .filter(|d| d.status.as_deref() != Some("failed"))
        .map(|d| DimensionInfo {
            strategy: d.strategy.clone(),
            player_count: d.player_count,
            depth_bb: d.depth_bb,
        })
        .collect();

    if !requested.is_empty() {
        dimensions.retain(|d| {
            let dim_ref = DimensionRef::new(&d.strategy, d.player_count, d.depth_bb);
            requested
                .iter()
                .any(|r| dimension_key(r) == dimension_key(&dim_ref))
        });
    }

    dimensions.sort_by(|a, b| {
        let ka = format!("{}:{}:{}", a.strategy, a.player_count, a.depth_bb);
        let kb = format!("{}:{}:{}", b.strategy, b.player_count, b.depth_bb);
        ka.cmp(&kb)
    });

    Ok(dimensions)
}

#[derive(Debug, Clone)]
pub(crate) struct DimensionInfo {
    pub(crate) strategy: String,
    pub(crate) player_count: u32,
    pub(crate) depth_bb: u32,
}

pub(crate) fn select_dimension_queries(
    source_db_path: &Path,
    dimensions: &[DimensionInfo],
    policy: QueryPolicy,
    fixed_line_id: Option<u32>,
    fixed_hand: Option<&str>,
) -> Result<Vec<DimensionQuery>, ToolError> {
    // For fixed policy, use the provided values.
    if policy == QueryPolicy::Fixed {
        let concrete_line_id = fixed_line_id.ok_or_else(|| {
            ToolError::invalid_argument("--query-policy fixed requires --concrete-line-id")
        })?;
        let hand = fixed_hand
            .ok_or_else(|| ToolError::invalid_argument("--query-policy fixed requires --hand"))?
            .to_owned();
        return Ok(dimensions
            .iter()
            .map(|d| DimensionQuery {
                strategy: d.strategy.clone(),
                player_count: d.player_count,
                depth_bb: d.depth_bb,
                concrete_line_id,
                hand: hand.clone(),
            })
            .collect());
    }

    // For "first" policy, pick the first row from each range_data table.
    let source_str = source_db_path
        .to_str()
        .ok_or_else(|| ToolError::invalid_argument("Source DB path is not valid UTF-8"))?;
    let mut queries = Vec::new();
    for dimension in dimensions {
        let query = pick_first_query(source_str, dimension)?;
        queries.push(query);
    }
    Ok(queries)
}

fn pick_first_query(
    source_db: &str,
    dimension: &DimensionInfo,
) -> Result<DimensionQuery, ToolError> {
    use range_store_core::sqlite::Connection;

    let conn = Connection::open(std::path::Path::new(source_db), true)?;
    let table = format!(
        "range_data_{}_{}max_{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    );
    let sql = format!(
        "SELECT concrete_line_id, hole_cards FROM \"{}\" ORDER BY concrete_line_id, id LIMIT 1",
        table.replace('"', "\"\"")
    );
    let mut stmt = conn.prepare(&sql)?;
    stmt.start(&[])?;
    if stmt.step_row()? {
        let concrete_line_id = u32::try_from(stmt.column_i64(0)).unwrap_or_default();
        let hand = stmt.column_text(1)?;
        return Ok(DimensionQuery {
            strategy: dimension.strategy.clone(),
            player_count: dimension.player_count,
            depth_bb: dimension.depth_bb,
            concrete_line_id,
            hand,
        });
    }
    Err(ToolError::new(
        "NOT_FOUND",
        format!("No rows in table {} for dimension query", table),
    ))
}

fn run_worker(
    worker_binary: &Path,
    command: &BenchmarkColdCommand,
    query: &DimensionQuery,
    run_index: usize,
    eviction: super::types::EvictionResult,
) -> ColdStartRunResult {
    let start = Instant::now();

    let mut cmd = Command::new(worker_binary);
    cmd.arg("cold-worker")
        .args(["--dir", &command.dir.to_string_lossy()])
        .args(["--meta", &command.meta.to_string_lossy()])
        .args(["--strategy", &query.strategy])
        .args(["--player-count", &query.player_count.to_string()])
        .args(["--depth-bb", &query.depth_bb.to_string()])
        .args(["--concrete-line-id", &query.concrete_line_id.to_string()])
        .args(["--hand", &query.hand]);
    if command.verify_checksums {
        cmd.arg("--verify-checksum");
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return failed_run(
                run_index,
                eviction,
                -1,
                &format!("Failed to spawn worker: {e}"),
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
                &format!("Failed to wait for worker: {e}"),
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
                "Worker did not return valid JSON. exitCode={exit_code}, stdout={}, stderr={}",
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

pub(crate) fn build_dimension_report(
    query: &DimensionQuery,
    results: &[ColdStartRunResult],
) -> DimensionColdStartReport {
    let dim_key = format!(
        "{}:{}:{}",
        query.strategy, query.player_count, query.depth_bb
    );
    let ok_results: Vec<&ColdStartRunResult> = results.iter().filter(|r| r.ok).collect();

    let failures: Vec<ColdStartRunFailure> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| ColdStartRunFailure {
            dimension: dim_key.clone(),
            run_index: r.run_index,
            exit_code: r.exit_code,
            error: r
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_owned()),
            valid_json: r.valid_json,
        })
        .collect();

    let memory_deltas: Vec<f64> = ok_results
        .iter()
        .filter_map(|r| {
            let before = r.memory_before.rss_bytes?;
            let after = r.memory_after.rss_bytes?;
            Some(after as f64 - before as f64)
        })
        .collect();

    let worst_accounting = ok_results
        .iter()
        .max_by(|a, b| {
            a.phase_accounting
                .unaccounted_ms
                .abs()
                .total_cmp(&b.phase_accounting.unaccounted_ms.abs())
        })
        .map(|r| r.phase_accounting.clone())
        .unwrap_or(PhaseAccounting::compute(&ColdWorkerTimings {
            service_open_ms: 0.0,
            dimension_prewarm_ms: 0.0,
            first_query_ms: 0.0,
            close_ms: 0.0,
            worker_total_ms: 0.0,
        }));

    DimensionColdStartReport {
        dimension: dim_key,
        query: query.clone(),
        runs: results.len(),
        success_count: ok_results.len(),
        error_count: failures.len(),
        store_open_and_first_query_ms: LatencySummary::from_values(
            &ok_results
                .iter()
                .map(|r| r.store_open_and_first_query_ms)
                .collect::<Vec<_>>(),
        ),
        process_elapsed_ms: LatencySummary::from_values(
            &ok_results
                .iter()
                .map(|r| r.process_elapsed_ms)
                .collect::<Vec<_>>(),
        ),
        phase_timings: ColdStartPhaseSummaries::from_results(
            &ok_results.iter().map(|r| (*r).clone()).collect::<Vec<_>>(),
        ),
        memory_delta_rss_bytes: LatencySummary::from_values(&memory_deltas),
        phase_accounting: worst_accounting,
        failures,
    }
}

fn build_report(
    command: &BenchmarkColdCommand,
    dimension_reports: &[DimensionColdStartReport],
    dataset_size_bytes: u64,
    filler_size_bytes: u64,
) -> ColdStartBenchmarkReport {
    // Flatten OK latencies across all dimensions for aggregate summaries.

    // Flatten OK latencies across dimensions
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

    // Build aggregate phase summaries from dimension-level summaries
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

    let notes = build_notes(command, dataset_size_bytes, filler_size_bytes);

    ColdStartBenchmarkReport {
        generated_at: crate::benchmark::report::generated_at_utc(),
        engine: "binary".to_owned(),
        mode: command.mode.to_string(),
        platform: std::env::consts::OS.to_owned(),
        runs_per_dimension: command.runs_per_dimension,
        source_db_path: command.source.to_string_lossy().to_string(),
        binary_dir: command.dir.to_string_lossy().to_string(),
        meta_db_path: command.meta.to_string_lossy().to_string(),
        verify_checksums: command.verify_checksums,
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
        notes,
    }
}

fn build_notes(
    command: &BenchmarkColdCommand,
    _dataset_size_bytes: u64,
    filler_size_bytes: u64,
) -> Vec<String> {
    let mut notes = vec![
        "Each run starts a fresh Rust process and records worker phase timings plus parent-observed process elapsed time.".to_owned(),
        "storeOpenAndFirstQueryMs = service open + dimension prewarm + first query. Use processElapsedMs or workerTotalMs for end-to-end cold start.".to_owned(),
        "Dimension prewarm opens/memmaps the dimension .idx/.bin files; action schemas are loaded lazily on the first query that uses each schema.".to_owned(),
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
    command: &BenchmarkColdCommand,
    report: &ColdStartBenchmarkReport,
) -> Result<(), ToolError> {
    write_cold_start_json(&command.out_path, report)?;
    write_cold_start_markdown(&command.md_path, report)?;
    Ok(())
}
