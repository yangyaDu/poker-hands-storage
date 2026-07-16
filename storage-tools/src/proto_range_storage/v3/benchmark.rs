use std::fs;
use std::path::{Path, PathBuf};

use range_store_core::dimension::{DimensionRef, DimensionSpec};
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::query::ActionFilter;
use range_store_core::sqlite::Connection;
use serde::Serialize;

use crate::benchmark::memory_snapshot::{get_memory_snapshot, BenchmarkMemoryReport};
use crate::benchmark::metrics::{measure_benchmark_case, BenchmarkCaseResult};
use crate::errors::ToolError;

use super::facade::{FacadeCacheStats, HandlePoolStats, V3Facade, V3FacadeOptions};
use super::manifest::{read_manifest, MANIFEST_FILE_NAME};
use super::query_service::V3QueryService;
use super::source::{load_metadata, load_strategy_rows};
use super::verification::{cross_verify_sqlite_v3, V3VerificationOptions};

#[derive(Debug, Clone)]
pub struct V3BenchmarkCommand {
    pub source_db: PathBuf,
    pub archive_root: PathBuf,
    pub dimension: DimensionSpec,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub max_open_handles: usize,
    pub metadata_cache_byte_budget_per_handle: usize,
    pub strategy_cache_byte_budget_per_handle: usize,
    pub verify_file_checksums: bool,
    pub out_path: PathBuf,
    pub markdown_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3BenchmarkReport {
    pub engine_pair: String,
    pub dimension: String,
    pub correctness_verified: bool,
    pub cases: Vec<BenchmarkCaseResult>,
    pub metadata_summary: V3BenchmarkSummary,
    pub strategy_summary: V3BenchmarkSummary,
    pub cache: FacadeCacheStats,
    pub handles: HandlePoolStats,
    pub memory: BenchmarkMemoryReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3BenchmarkSummary {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub qps: f64,
}

impl From<&BenchmarkCaseResult> for V3BenchmarkSummary {
    fn from(case: &BenchmarkCaseResult) -> Self {
        Self {
            p50_ms: case.p50_ms,
            p95_ms: case.p95_ms,
            p99_ms: case.p99_ms,
            qps: case.qps,
        }
    }
}

impl V3BenchmarkReport {
    pub fn has_errors(&self) -> bool {
        !self.correctness_verified || self.cases.iter().any(|case| case.error_count != 0)
    }
}

pub fn run_v3_benchmark(command: &V3BenchmarkCommand) -> Result<V3BenchmarkReport, ToolError> {
    if command.iterations == 0 || command.max_open_handles == 0 {
        return Err(ToolError::invalid_argument(
            "V3 benchmark iterations and max_open_handles must be positive",
        ));
    }
    let archive_dir = find_archive_dir(&command.archive_root, &command.dimension)?;
    let correctness = cross_verify_sqlite_v3(
        &command.source_db,
        &archive_dir,
        V3VerificationOptions::default(),
    );
    if !correctness.ok {
        return Err(ToolError::verify(format!(
            "V3 benchmark correctness gate failed with {} differences; first={:?}",
            correctness.failure_count,
            correctness.failure_samples.first()
        )));
    }

    let connection = Connection::open(&command.source_db, true)?;
    let loaded = load_metadata(&connection, &command.dimension)?;
    let first_path = loaded
        .concrete_paths
        .first()
        .ok_or_else(|| ToolError::verify("Benchmark dimension has no concrete paths"))?;
    let rows = load_strategy_rows(&connection, &command.dimension, first_path.source_id)?;
    let hand = rows
        .first()
        .map(|row| row.hole_cards.clone())
        .ok_or_else(|| ToolError::verify("Benchmark concrete path has no strategy rows"))?;
    let drill = loaded
        .drill_scenarios
        .first()
        .ok_or_else(|| ToolError::verify("Benchmark dimension has no drill scenarios"))?;
    let abstract_path = loaded
        .abstract_action_paths
        .first()
        .ok_or_else(|| ToolError::verify("Benchmark dimension has no abstract paths"))?;
    let dimension = DimensionRef::new(
        &command.dimension.strategy,
        command.dimension.player_count,
        command.dimension.depth_bb,
    );
    let items = vec![(); command.iterations];
    let cold_items = vec![(); command.iterations.clamp(1, 25)];
    let facade_options = V3FacadeOptions {
        max_open_handles: command.max_open_handles,
        verify_file_checksums: command.verify_file_checksums,
        metadata_cache_byte_budget_per_handle: command.metadata_cache_byte_budget_per_handle,
        strategy_cache_byte_budget_per_handle: command.strategy_cache_byte_budget_per_handle,
    };

    let memory_before = get_memory_snapshot();
    let mut cases = Vec::new();
    cases.push(measure_benchmark_case(
        "v3_cold_open",
        "Open and validate one V3 dimension handle",
        &cold_items,
        0,
        |_, _| {
            V3QueryService::open(&archive_dir)
                .map(|_| 1)
                .map_err(error_string)
        },
    ));
    cases.push(measure_benchmark_case(
        "v3_first_metadata_page",
        "Open a V3 handle and decode the first drill metadata page",
        &cold_items,
        0,
        |_, _| {
            let service = V3QueryService::open(&archive_dir).map_err(error_string)?;
            service
                .get_drill_scenario_lines(&dimension, &drill.drill_name)
                .map(|rows| rows.len())
                .map_err(error_string)
        },
    ));
    cases.push(measure_benchmark_case(
        "sqlite_metadata",
        "Load the dimension metadata from SQLite",
        &items,
        command.warmup_iterations,
        |_, _| {
            load_metadata(&connection, &command.dimension)
                .map(|metadata| metadata.concrete_paths.len())
                .map_err(error_string)
        },
    ));

    let facade = V3Facade::open_with_options(&command.archive_root, facade_options.clone())?;
    facade.prewarm(&dimension)?;
    cases.push(measure_benchmark_case(
        "v3_metadata_hit",
        "Read drill, abstract and concrete metadata through warm V3 page caches",
        &items,
        command.warmup_iterations,
        |_, index| match index % 3 {
            0 => facade
                .get_drill_scenario_lines(
                    &dimension.strategy,
                    &drill.drill_name,
                    dimension.player_count,
                    dimension.depth_bb,
                )
                .map(|rows| rows.len())
                .map_err(error_string),
            1 => facade
                .get_concrete_lines(
                    &dimension,
                    ConcreteLineFilter::Abstract(&abstract_path.abstract_action_path),
                )
                .map(|rows| rows.len())
                .map_err(error_string),
            _ => facade
                .get_concrete_lines(
                    &dimension,
                    ConcreteLineFilter::Concrete(&first_path.concrete_action_path),
                )
                .map(|rows| rows.len())
                .map_err(error_string),
        },
    ));
    cases.push(measure_benchmark_case(
        "v3_first_strategy_decode",
        "Open a V3 handle and decode one strategy payload",
        &cold_items,
        0,
        |_, _| {
            let service = V3QueryService::open(&archive_dir).map_err(error_string)?;
            service
                .query_hand_strategy(&dimension, first_path.concrete_action_path_id, &hand)
                .map(|result| result.actions.len())
                .map_err(error_string)
        },
    ));
    cases.push(measure_benchmark_case(
        "sqlite_strategy",
        "Read all source action cells for one SQLite concrete path",
        &items,
        command.warmup_iterations,
        |_, _| {
            load_strategy_rows(&connection, &command.dimension, first_path.source_id)
                .map(|rows| rows.len())
                .map_err(error_string)
        },
    ));
    cases.push(measure_benchmark_case(
        "v3_strategy_hit",
        "Query one hand through the warm V3 decoded-strategy cache",
        &items,
        command.warmup_iterations,
        |_, _| {
            facade
                .query_hand_strategy(&dimension, first_path.concrete_action_path_id, &hand)
                .map(|result| result.actions.len())
                .map_err(error_string)
        },
    ));
    let batch = vec![(first_path.concrete_action_path_id, hand.clone()); 8];
    cases.push(measure_benchmark_case(
        "v3_batch",
        "Query an eight-item V3 strategy batch",
        &items,
        command.warmup_iterations,
        |_, _| {
            facade
                .query_batch(&dimension, &batch)
                .map(|result| result.results.len())
                .map_err(error_string)
        },
    ));
    cases.push(measure_benchmark_case(
        "v3_hands_by_actions",
        "Scan 169 hands against V3 action filters",
        &items,
        command.warmup_iterations,
        |_, _| {
            facade
                .query_hands_by_actions(
                    &dimension,
                    first_path.concrete_action_path_id,
                    &[] as &[ActionFilter],
                    Some(0.0),
                )
                .map(|hands| hands.len())
                .map_err(error_string)
        },
    ));
    cases.push(measure_handle_reopen_case(
        command,
        &dimension,
        &items,
        &facade_options,
    ));

    let metadata_case = cases
        .iter()
        .find(|case| case.name == "v3_metadata_hit")
        .expect("V3 metadata benchmark case");
    let strategy_case = cases
        .iter()
        .find(|case| case.name == "v3_strategy_hit")
        .expect("V3 strategy benchmark case");
    let cache = facade.cache_stats();
    let handles = facade.handle_pool_stats();
    let memory_after = get_memory_snapshot();
    let report = V3BenchmarkReport {
        engine_pair: "sqlite-v3".to_owned(),
        dimension: format!(
            "{}:{}:{}",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        ),
        correctness_verified: true,
        metadata_summary: V3BenchmarkSummary::from(metadata_case),
        strategy_summary: V3BenchmarkSummary::from(strategy_case),
        cases,
        cache,
        handles,
        memory: BenchmarkMemoryReport::new(memory_before, memory_after),
    };
    write_report(command, &report)?;
    Ok(report)
}

fn measure_handle_reopen_case(
    command: &V3BenchmarkCommand,
    selected: &DimensionRef,
    items: &[()],
    options: &V3FacadeOptions,
) -> BenchmarkCaseResult {
    let other = discover_archive_dimensions(&command.archive_root)
        .into_iter()
        .find(|dimension| dimension != selected);
    let facade = V3Facade::open_with_options(
        &command.archive_root,
        V3FacadeOptions {
            max_open_handles: 1,
            ..options.clone()
        },
    );
    match (facade, other) {
        (Ok(facade), Some(other)) => measure_benchmark_case(
            "v3_handle_eviction_reopen",
            "Evict the selected dimension handle and reopen it",
            items,
            0,
            |_, _| {
                facade.prewarm(&other).map_err(error_string)?;
                facade.prewarm(selected).map_err(error_string)?;
                Ok(1)
            },
        ),
        _ => measure_benchmark_case(
            "v3_handle_reopen",
            "Reopen the only available V3 dimension handle",
            items,
            0,
            |_, _| {
                let facade = V3Facade::open_with_options(
                    &command.archive_root,
                    V3FacadeOptions {
                        max_open_handles: 1,
                        ..options.clone()
                    },
                )
                .map_err(error_string)?;
                facade.prewarm(selected).map_err(error_string)?;
                Ok(1)
            },
        ),
    }
}

fn find_archive_dir(root: &Path, dimension: &DimensionSpec) -> Result<PathBuf, ToolError> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if !path.join(MANIFEST_FILE_NAME).is_file() {
            continue;
        }
        let manifest = read_manifest(&path)?;
        if manifest.strategy == dimension.strategy
            && manifest.player_count == dimension.player_count
            && manifest.depth_bb == dimension.depth_bb
        {
            return Ok(path);
        }
    }
    Err(ToolError::new(
        "DIMENSION_NOT_FOUND",
        "Requested V3 benchmark dimension is not present",
    ))
}

fn discover_archive_dimensions(root: &Path) -> Vec<DimensionRef> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| read_manifest(&entry.path()).ok())
        .map(|manifest| {
            DimensionRef::new(manifest.strategy, manifest.player_count, manifest.depth_bb)
        })
        .collect()
}

fn write_report(command: &V3BenchmarkCommand, report: &V3BenchmarkReport) -> Result<(), ToolError> {
    if let Some(parent) = command.out_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = command.markdown_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &command.out_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(report)
                .map_err(|error| ToolError::invalid_format(error.to_string()))?
        ),
    )?;
    fs::write(
        &command.markdown_path,
        format!(
            "# SQLite / Proto V3 benchmark\n\n- Dimension: `{}`\n- Correctness: `{}`\n- Metadata P50/P95/P99: `{:.6}/{:.6}/{:.6} ms`\n- Strategy P50/P95/P99: `{:.6}/{:.6}/{:.6} ms`\n- Metadata cache resident bytes: `{}`\n- Strategy cache resident bytes: `{}`\n- RSS before/after: `{:?}/{:?}`\n",
            report.dimension,
            report.correctness_verified,
            report.metadata_summary.p50_ms,
            report.metadata_summary.p95_ms,
            report.metadata_summary.p99_ms,
            report.strategy_summary.p50_ms,
            report.strategy_summary.p95_ms,
            report.strategy_summary.p99_ms,
            report.cache.metadata.resident_estimated_bytes,
            report.cache.strategies.resident_estimated_bytes,
            report.memory.before.rss_bytes,
            report.memory.after.rss_bytes,
        ),
    )?;
    Ok(())
}

fn error_string(error: ToolError) -> String {
    error.to_string()
}
