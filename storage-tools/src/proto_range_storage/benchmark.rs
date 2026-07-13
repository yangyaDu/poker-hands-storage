use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use range_store_core::dimension::{DimensionRef, DimensionSpec};
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::query::StoreQueryService;
use serde::Serialize;

use crate::benchmark::cold::cache_eviction::{default_filler_size, evict_cache};
use crate::benchmark::cold::types::{
    ColdStartMode, ColdWorkerOutput, ColdWorkerTimings, LatencySummary,
};
use crate::benchmark::memory_snapshot::{get_memory_snapshot, MemorySnapshot};
use crate::benchmark::metrics::{measure_benchmark_case, safe_ratio, BenchmarkCaseResult};
use crate::benchmark::report_support::{
    format_ms, generated_at_utc, markdown_table, write_json_report, write_markdown_report,
};
use crate::errors::ToolError;

use super::line_matrix_store::{CompactArchiveOpenOptions, CompactLineMatrixArchive};

const HAND_COUNT_169: usize = 169;

#[derive(Debug, Clone)]
pub struct CompactVsCoreBenchmarkCommand {
    pub compact_dir: PathBuf,
    pub core_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub hot_iterations: usize,
    pub warmup_iterations: usize,
    pub cold_runs: usize,
    pub cold_mode: ColdStartMode,
    pub cache_filler_mb: Option<u64>,
    pub seed: u64,
    pub max_open_handles: usize,
    pub verify_checksums: bool,
    pub fixed_query: Option<CompactVsCoreQuery>,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreQuery {
    pub concrete_line_id: u64,
    pub hand_id: u8,
    pub hand: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactVsCoreEngine {
    Compact,
    Core,
}

impl CompactVsCoreEngine {
    pub fn parse(value: &str) -> Result<Self, ToolError> {
        match value {
            "compact" => Ok(Self::Compact),
            "core" => Ok(Self::Core),
            _ => Err(ToolError::invalid_argument(
                "--engine must be compact or core",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Core => "core",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompactVsCoreColdWorkerCommand {
    pub engine: CompactVsCoreEngine,
    pub compact_dir: PathBuf,
    pub core_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub query: CompactVsCoreQuery,
    pub max_open_handles: usize,
    pub verify_checksums: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreBenchmarkReport {
    pub generated_at: String,
    pub compact_archive_dir: PathBuf,
    pub core_dir: PathBuf,
    pub dimension: String,
    pub matrix_count: u64,
    pub workload: CompactVsCoreWorkloadSummary,
    pub hot: CompactVsCoreHotReport,
    pub cold: CompactVsCoreColdReport,
    pub notes: Vec<String>,
}

impl CompactVsCoreBenchmarkReport {
    pub fn has_errors(&self) -> bool {
        self.hot.compact.error_count > 0
            || self.hot.core.error_count > 0
            || self.cold.compact.error_count > 0
            || self.cold.core.error_count > 0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreWorkloadSummary {
    pub seed: u64,
    pub hot_iterations: usize,
    pub warmup_iterations: usize,
    pub cold_runs: usize,
    pub cold_mode: ColdStartMode,
    pub query_selection: String,
    pub cold_query: CompactVsCoreQuery,
    pub action_count_mismatch_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreHotReport {
    pub compact: BenchmarkCaseResult,
    pub core: BenchmarkCaseResult,
    pub compact_to_core_avg_ratio: f64,
    pub compact_to_core_p50_ratio: f64,
    pub compact_to_core_p95_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreColdReport {
    pub mode: ColdStartMode,
    pub combined_dataset_size_bytes: u64,
    pub cache_filler_size_bytes: u64,
    pub eviction_failure_count: usize,
    pub compact: CompactVsCoreColdEngineSummary,
    pub core: CompactVsCoreColdEngineSummary,
    pub compact_to_core_open_and_first_query_p50_ratio: f64,
    pub compact_to_core_open_and_first_query_p95_ratio: f64,
    pub compact_to_core_process_p50_ratio: f64,
    pub compact_to_core_process_p95_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreColdEngineSummary {
    pub successful_runs: usize,
    pub error_count: usize,
    pub result_count: u64,
    pub store_open_and_first_query_ms: LatencySummary,
    pub process_elapsed_ms: LatencySummary,
    pub timings: CompactVsCoreColdTimingSummary,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactVsCoreColdTimingSummary {
    pub open_ms: LatencySummary,
    pub prewarm_ms: LatencySummary,
    pub first_query_ms: LatencySummary,
    pub close_ms: LatencySummary,
    pub worker_total_ms: LatencySummary,
    pub process_overhead_ms: LatencySummary,
}

#[derive(Debug, Clone)]
struct ColdRun {
    ok: bool,
    result_count: usize,
    store_open_and_first_query_ms: f64,
    process_elapsed_ms: f64,
    timings: ColdWorkerTimings,
    error: Option<String>,
}

pub fn run_compact_vs_core_benchmark(
    command: &CompactVsCoreBenchmarkCommand,
) -> Result<CompactVsCoreBenchmarkReport, ToolError> {
    if command.hot_iterations == 0 {
        return Err(ToolError::invalid_argument(
            "--hot-iterations must be at least 1",
        ));
    }
    if command.cold_runs == 0 {
        return Err(ToolError::invalid_argument(
            "--cold-runs must be at least 1",
        ));
    }
    if command.max_open_handles == 0 {
        return Err(ToolError::invalid_argument(
            "--max-open-handles must be at least 1",
        ));
    }

    let compact_archive = CompactLineMatrixArchive::open_with_options(
        &command.compact_dir,
        CompactArchiveOpenOptions {
            verify_checksums: command.verify_checksums,
            cache_capacity: 4096,
        },
    )?;
    assert_archive_dimension(&compact_archive, &command.dimension)?;
    let queries = build_query_plan(&compact_archive, command)?;
    let cold_query = queries
        .first()
        .cloned()
        .ok_or_else(|| ToolError::invalid_format("Compact benchmark query plan is empty"))?;
    let dimension_ref = DimensionRef::new(
        &command.dimension.strategy,
        command.dimension.player_count,
        command.dimension.depth_bb,
    );
    let core_service = StoreQueryService::open_with_meta(
        &command.core_dir,
        command.core_dir.join("meta.db"),
        command.max_open_handles,
        command.verify_checksums,
    )?;
    core_service.prewarm(&dimension_ref)?;

    // Query-plan construction decodes CompactLineMatrix records to choose valid hands.
    // Execute the same plan through both readers before timing so both operate hot.
    let action_count_mismatch_count =
        warmup_query_plan(&compact_archive, &core_service, &dimension_ref, &queries)?;

    let compact_hot = measure_benchmark_case(
        "compact-line-matrix:hand-strategy",
        "Decode one CompactLineMatrix and return materialized action values for one hand.",
        &queries,
        command.warmup_iterations,
        |query, _| compact_action_count(&compact_archive, query).map_err(|error| error.to_string()),
    );
    let core_hot = measure_benchmark_case(
        "range-strata-core:hand-strategy",
        "Query one concrete_line_id + hand through the core .bin/.idx reader.",
        &queries,
        command.warmup_iterations,
        |query, _| {
            core_action_count(&core_service, &dimension_ref, query)
                .map_err(|error| error.to_string())
        },
    );

    let cold = run_cold_benchmark(command, &cold_query)?;
    let report = CompactVsCoreBenchmarkReport {
        generated_at: generated_at_utc(),
        compact_archive_dir: command.compact_dir.clone(),
        core_dir: command.core_dir.clone(),
        dimension: dimension_label(&command.dimension),
        matrix_count: compact_archive.matrix_count(),
        workload: CompactVsCoreWorkloadSummary {
            seed: command.seed,
            hot_iterations: command.hot_iterations,
            warmup_iterations: command.warmup_iterations,
            cold_runs: command.cold_runs,
            cold_mode: command.cold_mode,
            query_selection: if command.fixed_query.is_some() {
                "fixed".to_owned()
            } else {
                "deterministic-random-valid-hand".to_owned()
            },
            cold_query,
            action_count_mismatch_count,
        },
        hot: CompactVsCoreHotReport {
            compact_to_core_avg_ratio: safe_ratio(compact_hot.avg_ms, core_hot.avg_ms),
            compact_to_core_p50_ratio: safe_ratio(compact_hot.p50_ms, core_hot.p50_ms),
            compact_to_core_p95_ratio: safe_ratio(compact_hot.p95_ms, core_hot.p95_ms),
            compact: compact_hot,
            core: core_hot,
        },
        cold,
        notes: vec![
            "Hot measurements exclude deterministic query-plan construction and execute an untimed symmetry warm-up through both engines.".to_owned(),
            "Compact result_count counts action values materialized for the requested hand; core result_count counts actions returned by the core hand-strategy API. Proto excludes rows with hand_ev IS NULL, so result counts are informational rather than a cross-format equality gate.".to_owned(),
            "Cold runs use a fresh worker process per engine/run. process-cold refreshes process state but does not evict the OS page cache; the configured filler is used only by cache-eviction modes.".to_owned(),
            "Compact/core ratios below 1.0 mean CompactLineMatrix was faster for that metric.".to_owned(),
        ],
    };
    write_json_report(&command.out_path, &report)?;
    write_markdown_report(&command.md_path, render_markdown(&report))?;
    Ok(report)
}

pub fn run_compact_vs_core_cold_worker(
    command: &CompactVsCoreColdWorkerCommand,
) -> ColdWorkerOutput {
    let worker_start = Instant::now();
    let mut timings = empty_timings();
    let result = match command.engine {
        CompactVsCoreEngine::Compact => run_compact_cold_worker(command, &mut timings),
        CompactVsCoreEngine::Core => run_core_cold_worker(command, &mut timings),
    };
    timings.worker_total_ms = elapsed_ms(worker_start);

    match result {
        Ok((result_count, memory_before, memory_after)) => ColdWorkerOutput {
            ok: true,
            store_open_and_first_query_ms: timings.service_open_ms
                + timings.dimension_prewarm_ms
                + timings.first_query_ms,
            result_count,
            memory_before,
            memory_after,
            timings,
            error: None,
        },
        Err(error) => ColdWorkerOutput {
            ok: false,
            store_open_and_first_query_ms: timings.service_open_ms
                + timings.dimension_prewarm_ms
                + timings.first_query_ms,
            result_count: 0,
            memory_before: empty_snapshot(),
            memory_after: empty_snapshot(),
            timings,
            error: Some(error.to_string()),
        },
    }
}

fn run_compact_cold_worker(
    command: &CompactVsCoreColdWorkerCommand,
    timings: &mut ColdWorkerTimings,
) -> Result<(usize, MemorySnapshot, MemorySnapshot), ToolError> {
    let open_start = Instant::now();
    let archive = CompactLineMatrixArchive::open_with_options(
        &command.compact_dir,
        CompactArchiveOpenOptions {
            verify_checksums: command.verify_checksums,
            cache_capacity: 4096,
        },
    )?;
    assert_archive_dimension(&archive, &command.dimension)?;
    timings.service_open_ms = elapsed_ms(open_start);

    let memory_before = get_memory_snapshot();
    let query_start = Instant::now();
    let result_count = compact_action_count(&archive, &command.query)?;
    timings.first_query_ms = elapsed_ms(query_start);
    let memory_after = get_memory_snapshot();

    let close_start = Instant::now();
    drop(archive);
    timings.close_ms = elapsed_ms(close_start);
    Ok((result_count, memory_before, memory_after))
}

fn run_core_cold_worker(
    command: &CompactVsCoreColdWorkerCommand,
    timings: &mut ColdWorkerTimings,
) -> Result<(usize, MemorySnapshot, MemorySnapshot), ToolError> {
    let open_start = Instant::now();
    let service = StoreQueryService::open_with_meta(
        &command.core_dir,
        command.core_dir.join("meta.db"),
        command.max_open_handles,
        command.verify_checksums,
    )?;
    timings.service_open_ms = elapsed_ms(open_start);

    let memory_before = get_memory_snapshot();
    let dimension = DimensionRef::new(
        &command.dimension.strategy,
        command.dimension.player_count,
        command.dimension.depth_bb,
    );
    let prewarm_start = Instant::now();
    service.prewarm(&dimension)?;
    timings.dimension_prewarm_ms = elapsed_ms(prewarm_start);

    let query_start = Instant::now();
    let result_count = core_action_count(&service, &dimension, &command.query)?;
    timings.first_query_ms = elapsed_ms(query_start);
    let memory_after = get_memory_snapshot();

    let close_start = Instant::now();
    drop(service);
    timings.close_ms = elapsed_ms(close_start);
    Ok((result_count, memory_before, memory_after))
}

fn build_query_plan(
    archive: &CompactLineMatrixArchive,
    command: &CompactVsCoreBenchmarkCommand,
) -> Result<Vec<CompactVsCoreQuery>, ToolError> {
    if let Some(query) = &command.fixed_query {
        validate_query(archive, query)?;
        return Ok(vec![query.clone(); command.hot_iterations]);
    }

    let mut state = command.seed.max(1);
    let mut queries = Vec::with_capacity(command.hot_iterations);
    for _ in 0..command.hot_iterations {
        let concrete_line_id = next_random(&mut state) % archive.matrix_count() + 1;
        let matrix = archive.read_matrix(concrete_line_id)?;
        let valid_hands = valid_hand_ids(matrix.matrix());
        let hand_id = *valid_hands
            .get((next_random(&mut state) as usize) % valid_hands.len())
            .ok_or_else(|| {
                ToolError::invalid_format(format!(
                    "Compact matrix {concrete_line_id} has no valid hands for benchmark"
                ))
            })?;
        queries.push(CompactVsCoreQuery {
            concrete_line_id,
            hand_id,
            hand: hand_code_from_id(hand_id),
        });
    }
    Ok(queries)
}

fn validate_query(
    archive: &CompactLineMatrixArchive,
    query: &CompactVsCoreQuery,
) -> Result<(), ToolError> {
    if usize::from(query.hand_id) >= HAND_COUNT_169 {
        return Err(ToolError::invalid_argument("--hand-id must be in 0..=168"));
    }
    let matrix = archive.read_matrix(query.concrete_line_id)?;
    if !is_valid_hand(matrix.matrix(), usize::from(query.hand_id)) {
        return Err(ToolError::invalid_argument(format!(
            "Hand {} is not present in compact line {}",
            query.hand, query.concrete_line_id
        )));
    }
    Ok(())
}

fn warmup_query_plan(
    archive: &CompactLineMatrixArchive,
    core: &StoreQueryService,
    dimension: &DimensionRef,
    queries: &[CompactVsCoreQuery],
) -> Result<usize, ToolError> {
    let mut mismatch_count = 0;
    for query in queries {
        let compact_count = compact_action_count(archive, query)?;
        let core_count = core_action_count(core, dimension, query)?;
        if compact_count != core_count {
            mismatch_count += 1;
        }
    }
    Ok(mismatch_count)
}

fn compact_action_count(
    archive: &CompactLineMatrixArchive,
    query: &CompactVsCoreQuery,
) -> Result<usize, ToolError> {
    let matrix = archive.read_matrix(query.concrete_line_id)?;
    Ok(matrix
        .matrix()
        .actions
        .iter()
        .enumerate()
        .filter(|(action_index, _)| {
            matrix
                .action_value(*action_index, usize::from(query.hand_id))
                .is_some()
        })
        .count())
}

fn core_action_count(
    service: &StoreQueryService,
    dimension: &DimensionRef,
    query: &CompactVsCoreQuery,
) -> Result<usize, ToolError> {
    let concrete_line_id = u32::try_from(query.concrete_line_id)
        .map_err(|_| ToolError::invalid_argument("concrete_line_id exceeds core u32 range"))?;
    service
        .query(dimension, concrete_line_id, &query.hand)
        .map(|result| result.actions.len())
        .map_err(|error| ToolError::new("CORE_COMPACT_BENCHMARK_QUERY", error.to_string()))
}

fn run_cold_benchmark(
    command: &CompactVsCoreBenchmarkCommand,
    query: &CompactVsCoreQuery,
) -> Result<CompactVsCoreColdReport, ToolError> {
    let worker_binary = std::env::current_exe().map_err(|error| {
        ToolError::new(
            "CURRENT_EXE",
            format!("Cannot determine benchmark executable path: {error}"),
        )
    })?;
    let combined_dataset_size_bytes = directory_size(&command.compact_dir)
        .checked_add(directory_size(&command.core_dir))
        .ok_or_else(|| ToolError::invalid_format("Compact/core dataset size overflow"))?;
    let cache_filler_size_bytes = command
        .cache_filler_mb
        .map(|value| value.saturating_mul(1024 * 1024))
        .unwrap_or_else(|| default_filler_size(combined_dataset_size_bytes));
    let mut compact_runs = Vec::with_capacity(command.cold_runs);
    let mut core_runs = Vec::with_capacity(command.cold_runs);
    let mut eviction_failure_count = 0;

    for run_index in 0..command.cold_runs {
        let engines = if run_index % 2 == 0 {
            [CompactVsCoreEngine::Compact, CompactVsCoreEngine::Core]
        } else {
            [CompactVsCoreEngine::Core, CompactVsCoreEngine::Compact]
        };
        for engine in engines {
            let eviction = evict_cache(
                command.cold_mode,
                cache_filler_size_bytes,
                combined_dataset_size_bytes,
            );
            if !eviction.succeeded {
                eviction_failure_count += 1;
            }
            let run = run_cold_worker_process(&worker_binary, command, query, engine);
            match engine {
                CompactVsCoreEngine::Compact => compact_runs.push(run),
                CompactVsCoreEngine::Core => core_runs.push(run),
            }
        }
    }

    let compact = summarize_cold_runs(&compact_runs);
    let core = summarize_cold_runs(&core_runs);
    Ok(CompactVsCoreColdReport {
        mode: command.cold_mode,
        combined_dataset_size_bytes,
        cache_filler_size_bytes,
        eviction_failure_count,
        compact_to_core_open_and_first_query_p50_ratio: safe_ratio(
            compact.store_open_and_first_query_ms.p50_ms,
            core.store_open_and_first_query_ms.p50_ms,
        ),
        compact_to_core_open_and_first_query_p95_ratio: safe_ratio(
            compact.store_open_and_first_query_ms.p95_ms,
            core.store_open_and_first_query_ms.p95_ms,
        ),
        compact_to_core_process_p50_ratio: safe_ratio(
            compact.process_elapsed_ms.p50_ms,
            core.process_elapsed_ms.p50_ms,
        ),
        compact_to_core_process_p95_ratio: safe_ratio(
            compact.process_elapsed_ms.p95_ms,
            core.process_elapsed_ms.p95_ms,
        ),
        compact,
        core,
    })
}

fn run_cold_worker_process(
    worker_binary: &Path,
    command: &CompactVsCoreBenchmarkCommand,
    query: &CompactVsCoreQuery,
    engine: CompactVsCoreEngine,
) -> ColdRun {
    let start = Instant::now();
    let output = Command::new(worker_binary)
        .arg("compact-vs-core-cold-worker")
        .args(["--engine", engine.as_str()])
        .args(["--compact-dir", &command.compact_dir.to_string_lossy()])
        .args(["--core-dir", &command.core_dir.to_string_lossy()])
        .args(["--dimension", &dimension_label(&command.dimension)])
        .args(["--concrete-line-id", &query.concrete_line_id.to_string()])
        .args(["--hand-id", &query.hand_id.to_string()])
        .args(["--max-open-handles", &command.max_open_handles.to_string()])
        .arg(if command.verify_checksums {
            "--verify-checksum"
        } else {
            "--no-verify-checksum"
        })
        .output();
    let process_elapsed_ms = elapsed_ms(start);
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return failed_cold_run(
                process_elapsed_ms,
                format!("Failed to spawn worker: {error}"),
            )
        }
    };
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let parsed = match serde_json::from_str::<ColdWorkerOutput>(&stdout) {
        Ok(parsed) => parsed,
        Err(_) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return failed_cold_run(
                process_elapsed_ms,
                format!(
                    "Worker returned invalid JSON. exitCode={exit_code}, stdout={}, stderr={}",
                    &stdout[..stdout.len().min(500)],
                    &stderr[..stderr.len().min(500)]
                ),
            );
        }
    };
    ColdRun {
        ok: parsed.ok && output.status.success(),
        result_count: parsed.result_count,
        store_open_and_first_query_ms: parsed.store_open_and_first_query_ms,
        process_elapsed_ms,
        timings: parsed.timings,
        error: if parsed.ok && output.status.success() {
            None
        } else {
            parsed
                .error
                .or_else(|| Some(format!("Worker exited with code {exit_code}")))
        },
    }
}

fn failed_cold_run(process_elapsed_ms: f64, error: String) -> ColdRun {
    ColdRun {
        ok: false,
        result_count: 0,
        store_open_and_first_query_ms: 0.0,
        process_elapsed_ms,
        timings: empty_timings(),
        error: Some(error),
    }
}

fn summarize_cold_runs(runs: &[ColdRun]) -> CompactVsCoreColdEngineSummary {
    let successful = runs.iter().filter(|run| run.ok).collect::<Vec<_>>();
    let values =
        |select: fn(&ColdRun) -> f64| successful.iter().map(|run| select(run)).collect::<Vec<_>>();
    let timing_values = |select: fn(&ColdWorkerTimings) -> f64| {
        successful
            .iter()
            .map(|run| select(&run.timings))
            .collect::<Vec<_>>()
    };
    CompactVsCoreColdEngineSummary {
        successful_runs: successful.len(),
        error_count: runs.len().saturating_sub(successful.len()),
        result_count: successful.iter().map(|run| run.result_count as u64).sum(),
        store_open_and_first_query_ms: LatencySummary::from_values(&values(|run| {
            run.store_open_and_first_query_ms
        })),
        process_elapsed_ms: LatencySummary::from_values(&values(|run| run.process_elapsed_ms)),
        timings: CompactVsCoreColdTimingSummary {
            open_ms: LatencySummary::from_values(&timing_values(|timings| timings.service_open_ms)),
            prewarm_ms: LatencySummary::from_values(&timing_values(|timings| {
                timings.dimension_prewarm_ms
            })),
            first_query_ms: LatencySummary::from_values(&timing_values(|timings| {
                timings.first_query_ms
            })),
            close_ms: LatencySummary::from_values(&timing_values(|timings| timings.close_ms)),
            worker_total_ms: LatencySummary::from_values(&timing_values(|timings| {
                timings.worker_total_ms
            })),
            process_overhead_ms: LatencySummary::from_values(&values(|run| {
                (run.process_elapsed_ms - run.timings.worker_total_ms).max(0.0)
            })),
        },
        first_error: runs.iter().find_map(|run| run.error.clone()),
    }
}

fn render_markdown(report: &CompactVsCoreBenchmarkReport) -> String {
    let mut markdown = String::from("# Proto LineMatrix vs Core Binary Benchmark Report\n\n");
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str("## Scope\n\n");
    markdown.push_str(&format!("- Dimension: `{}`\n", report.dimension));
    markdown.push_str(&format!(
        "- Compact archive: `{}`\n",
        report.compact_archive_dir.display()
    ));
    markdown.push_str(&format!(
        "- Core data directory: `{}`\n",
        report.core_dir.display()
    ));
    markdown.push_str(&format!("- Compact matrices: {}\n", report.matrix_count));
    markdown.push_str(&format!(
        "- Query selection: {}\n",
        report.workload.query_selection
    ));
    markdown.push_str(&format!(
        "- Cold query: line {} / hand {} (id {})\n\n",
        report.workload.cold_query.concrete_line_id,
        report.workload.cold_query.hand,
        report.workload.cold_query.hand_id
    ));

    markdown.push_str("## Hot Query\n\n");
    markdown.push_str(&markdown_table(
        &[
            "Engine",
            "Avg",
            "P50",
            "P95",
            "P99",
            "QPS",
            "Errors",
            "Result values",
        ],
        &[
            hot_row("Proto LineMatrix", &report.hot.compact),
            hot_row("Core .bin/.idx", &report.hot.core),
        ],
    ));
    markdown.push_str(&format!(
        "\nCompact/Core ratio: avg {:.2}x, P50 {:.2}x, P95 {:.2}x. Values below 1.0 favor CompactLineMatrix.\n\n",
        report.hot.compact_to_core_avg_ratio,
        report.hot.compact_to_core_p50_ratio,
        report.hot.compact_to_core_p95_ratio,
    ));

    markdown.push_str("## Cold Start\n\n");
    markdown.push_str(&format!(
        "- Mode: `{}`; runs per engine: {}; configured cache filler: {}\n",
        report.cold.mode,
        report.workload.cold_runs,
        format_binary_bytes(report.cold.cache_filler_size_bytes),
    ));
    markdown.push_str(&markdown_table(
        &[
            "Engine",
            "Open + first P50",
            "Open + first P95",
            "Process P50",
            "Process P95",
            "First query P95",
            "Errors",
        ],
        &[
            cold_row("Proto LineMatrix", &report.cold.compact),
            cold_row("Core .bin/.idx", &report.cold.core),
        ],
    ));
    markdown.push_str(&format!(
        "\nCompact/Core ratio: open+first P50 {:.2}x, P95 {:.2}x; process P50 {:.2}x, P95 {:.2}x.\n\n",
        report.cold.compact_to_core_open_and_first_query_p50_ratio,
        report.cold.compact_to_core_open_and_first_query_p95_ratio,
        report.cold.compact_to_core_process_p50_ratio,
        report.cold.compact_to_core_process_p95_ratio,
    ));

    markdown.push_str("## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn hot_row(name: &str, result: &BenchmarkCaseResult) -> Vec<String> {
    vec![
        name.to_owned(),
        format_ms(result.avg_ms),
        format_ms(result.p50_ms),
        format_ms(result.p95_ms),
        format_ms(result.p99_ms),
        format!("{:.2}", result.qps),
        result.error_count.to_string(),
        result.result_count.to_string(),
    ]
}

fn cold_row(name: &str, result: &CompactVsCoreColdEngineSummary) -> Vec<String> {
    vec![
        name.to_owned(),
        format_ms(result.store_open_and_first_query_ms.p50_ms),
        format_ms(result.store_open_and_first_query_ms.p95_ms),
        format_ms(result.process_elapsed_ms.p50_ms),
        format_ms(result.process_elapsed_ms.p95_ms),
        format_ms(result.timings.first_query_ms.p95_ms),
        result.error_count.to_string(),
    ]
}

fn assert_archive_dimension(
    archive: &CompactLineMatrixArchive,
    expected: &DimensionSpec,
) -> Result<(), ToolError> {
    let actual = archive.dimension();
    if actual.strategy != expected.strategy
        || actual.player_count != expected.player_count
        || actual.depth_bb != expected.depth_bb
    {
        return Err(ToolError::invalid_argument(format!(
            "Compact archive dimension {} does not match requested {}",
            dimension_label(actual),
            dimension_label(expected)
        )));
    }
    Ok(())
}

fn valid_hand_ids(matrix: &crate::proto_range_storage::proto::CompactLineMatrix) -> Vec<u8> {
    (0..HAND_COUNT_169)
        .filter(|hand_id| is_valid_hand(matrix, *hand_id))
        .map(|hand_id| hand_id as u8)
        .collect()
}

fn is_valid_hand(
    matrix: &crate::proto_range_storage::proto::CompactLineMatrix,
    hand_id: usize,
) -> bool {
    matrix
        .valid_hand_bitmap
        .get(hand_id / 8)
        .is_some_and(|byte| byte & (1 << (hand_id % 8)) != 0)
}

fn next_random(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

fn dimension_label(dimension: &DimensionSpec) -> String {
    format!(
        "{}:{}:{}",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

fn directory_size(path: &Path) -> u64 {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return 0,
    };
    if metadata.is_file() {
        return metadata.len();
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    entries
        .flatten()
        .map(|entry| directory_size(&entry.path()))
        .sum()
}

fn empty_timings() -> ColdWorkerTimings {
    ColdWorkerTimings {
        service_open_ms: 0.0,
        dimension_prewarm_ms: 0.0,
        first_query_ms: 0.0,
        close_ms: 0.0,
        worker_total_ms: 0.0,
    }
}

fn empty_snapshot() -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: None,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some("Worker failed before memory snapshot.".to_owned()),
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn format_binary_bytes(value: u64) -> String {
    if value >= 1024 * 1024 {
        format!("{:.1} MiB", value as f64 / (1024.0 * 1024.0))
    } else if value >= 1024 {
        format!("{:.1} KiB", value as f64 / 1024.0)
    } else {
        format!("{value} B")
    }
}
