use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use range_store_core::dimension::{get_concrete_lines_table_name, quote_identifier, DimensionRef};
use range_store_core::sqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::benchmark::cli::{next_value, parse_usize, parse_usize_list};
use crate::benchmark::cold::types::LatencySummary;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, MemorySnapshot};
use crate::benchmark::report_support::{
    generated_at_utc, markdown_table, write_json_report, write_markdown_report,
};
use crate::benchmark::types::{
    BenchmarkWorkload, DrillScenarioBenchmarkItem, HandBenchmarkItem, WorkloadMode, WorkloadOptions,
};
use crate::benchmark::workload::{create_benchmark_workload, read_workload_json};
use crate::errors::ToolError;

use super::cli::parse_three_way_hot_benchmark_args;
use super::query_facade::{ProtoRangeStoreFacade, ProtoRangeStoreFacadeOptions};
use super::three_way_benchmark::{
    run_three_way_hot_benchmark, run_three_way_hot_benchmark_with_workload,
    ThreeWayHotBenchmarkCommand, ThreeWayHotBenchmarkReport,
};

#[derive(Debug, Clone)]
pub struct ThreeWayStabilityBenchmarkCommand {
    pub hot: ThreeWayHotBenchmarkCommand,
    pub runs: usize,
    pub matrix_cache_capacities: Vec<usize>,
    pub matrix_cache_byte_budgets: Vec<Option<usize>>,
    pub line_transition_start: Option<String>,
    pub line_transition_sessions: usize,
    pub line_transition_replay_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayStabilityBenchmarkReport {
    pub generated_at: String,
    pub runs: usize,
    pub raw_report_paths: Vec<String>,
    pub cases: Vec<ThreeWayStabilityCase>,
    pub metadata_cache: Option<MetadataCachePhaseReport>,
    pub hand_strategy_profile: ProtoHandStrategyProfileReport,
    pub matrix_cache_sweep: Vec<MatrixCacheSweepReport>,
    pub line_transition_sweep: Option<LineTransitionSweepReport>,
    pub line_transition_replay_sweep: Option<LineTransitionReplaySweepReport>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayStabilityCase {
    pub name: String,
    pub proto_avg_ms: LatencySummary,
    pub sqlite_avg_ms: LatencySummary,
    pub proto_to_sqlite_avg_ratio: LatencySummary,
    pub core_avg_ms: LatencySummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataCachePhaseReport {
    pub target: DrillScenarioBenchmarkItem,
    pub eviction_target: Option<DrillScenarioBenchmarkItem>,
    pub first_query_ms: f64,
    pub cache_hit_ms: f64,
    pub post_eviction_query_ms: Option<f64>,
    pub first_query_rss_delta_bytes: Option<i64>,
    pub cache_hit_rss_delta_bytes: Option<i64>,
    pub post_eviction_rss_delta_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtoHandStrategyProfileReport {
    pub cache_config: MatrixCacheConfig,
    pub warmup_requests: usize,
    pub samples: usize,
    pub facade_total_ms: LatencySummary,
    pub facade_overhead_ms: LatencySummary,
    pub dimension_check_ms: LatencySummary,
    pub parse_hand_ms: LatencySummary,
    pub matrix_read_ms: LatencySummary,
    pub matrix_cache_hits: usize,
    pub matrix_cache_misses: usize,
    pub matrix_first_seen_misses: usize,
    pub matrix_revisit_after_eviction_misses: usize,
    pub unique_matrix_count: usize,
    pub reuse_distance: Option<LatencySummary>,
    pub matrix_cache_evictions: u64,
    pub matrix_cache_oversized_skips: u64,
    pub max_observed_resident_estimated_bytes: usize,
    pub max_observed_peak_resident_estimated_bytes: usize,
    pub dimension_handle_opens: u64,
    pub dimension_handle_evictions: u64,
    pub matrix_index_payload_ms: LatencySummary,
    pub matrix_protobuf_decode_ms: LatencySummary,
    pub matrix_compact_index_ms: LatencySummary,
    pub matrix_cache_insert_ms: LatencySummary,
    pub action_materialization_ms: LatencySummary,
    pub service_total_ms: LatencySummary,
    pub slowest: Vec<SlowHandStrategyProfile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowHandStrategyProfile {
    pub dimension: String,
    pub concrete_line_id: u32,
    pub hand: String,
    pub facade_total_ms: f64,
    pub matrix_read_ms: f64,
    pub action_materialization_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixCacheConfig {
    pub entry_capacity: usize,
    pub byte_budget_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixCacheSweepReport {
    pub config: MatrixCacheConfig,
    pub hand_strategy_profile: ProtoHandStrategyProfileReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineTransitionSweepReport {
    pub dimension: String,
    pub start_concrete_line: String,
    pub sessions: usize,
    pub steps: usize,
    pub candidate_leaf_count: usize,
    pub implicit_fold_normalized_prefix_count: usize,
    pub skipped_unresolvable_leaf_count: usize,
    pub skipped_no_retained_hand_leaf_count: usize,
    pub child_fanout: Option<LatencySummary>,
    pub cache_configs: Vec<MatrixCacheSweepReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineTransitionReplaySweepReport {
    pub replay_path: String,
    pub sessions: usize,
    pub steps: usize,
    pub dimensions: Vec<String>,
    pub cache_configs: Vec<LineTransitionReplayCacheConfigReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineTransitionReplayCacheConfigReport {
    pub config: MatrixCacheConfig,
    pub session_total_ms: LatencySummary,
    pub hand_strategy_profile: ProtoHandStrategyProfileReport,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalLineReplay {
    schema_version: u32,
    sessions: Vec<CanonicalLineReplaySession>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalLineReplaySession {
    name: String,
    requests: Vec<HandBenchmarkItem>,
}

#[derive(Debug, Clone)]
struct LineTransitionSourceLine {
    concrete_line_id: u32,
    concrete_line: String,
}

#[derive(Debug, Clone)]
struct LineTransitionWorkload {
    dimension: DimensionRef,
    start_concrete_line: String,
    sessions: usize,
    requests: Vec<HandBenchmarkItem>,
    child_fanout: Vec<f64>,
    candidate_leaf_count: usize,
    implicit_fold_normalized_prefix_count: usize,
    skipped_unresolvable_leaf_count: usize,
    skipped_no_retained_hand_leaf_count: usize,
}

#[derive(Debug, Clone)]
struct LineTransitionReplayWorkload {
    replay_path: PathBuf,
    sessions: Vec<CanonicalLineReplaySession>,
    requests: Vec<HandBenchmarkItem>,
    dimensions: Vec<String>,
}

impl LineTransitionReplayWorkload {
    fn as_benchmark_workload(&self, seed: u64) -> BenchmarkWorkload {
        BenchmarkWorkload {
            seed,
            mode: WorkloadMode::Random,
            dimensions: self.dimensions.clone(),
            hand_queries: self.requests.clone(),
            batch_queries: Vec::new(),
            batch_size: 0,
            batch_queries_by_size: Vec::new(),
            hands_by_actions_queries: Vec::new(),
            drill_scenario_queries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkloadMatrixKey {
    dimension: String,
    concrete_line_id: u32,
}

pub fn parse_three_way_stability_benchmark_args(
    args: Vec<String>,
) -> Result<ThreeWayStabilityBenchmarkCommand, ToolError> {
    let mut runs = 3usize;
    let mut matrix_cache_capacities = vec![1024usize];
    let mut matrix_cache_byte_budgets = vec![None];
    let mut line_transition_start = None;
    let mut line_transition_sessions = 0usize;
    let mut line_transition_replay_path = None;
    let mut hot_args = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--runs" => runs = parse_usize("--runs", next_value(&args, &mut index)?)?,
            "--matrix-cache-capacities" => {
                matrix_cache_capacities =
                    parse_usize_list("--matrix-cache-capacities", next_value(&args, &mut index)?)?
            }
            "--matrix-cache-byte-budgets" => {
                matrix_cache_byte_budgets =
                    parse_cache_byte_budgets(next_value(&args, &mut index)?)?
            }
            "--line-transition-start" => {
                line_transition_start = Some(next_value(&args, &mut index)?.to_owned())
            }
            "--line-transition-sessions" => {
                line_transition_sessions =
                    parse_usize("--line-transition-sessions", next_value(&args, &mut index)?)?
            }
            "--line-transition-replay" => {
                line_transition_replay_path = Some(PathBuf::from(next_value(&args, &mut index)?))
            }
            _ => hot_args.push(args[index].clone()),
        }
        index += 1;
    }
    if runs < 2 {
        return Err(ToolError::invalid_argument("--runs must be at least 2"));
    }
    if line_transition_sessions > 0 && line_transition_start.is_none() {
        return Err(ToolError::invalid_argument(
            "--line-transition-start is required when --line-transition-sessions is set",
        ));
    }
    if line_transition_replay_path.is_some()
        && (line_transition_start.is_some() || line_transition_sessions > 0)
    {
        return Err(ToolError::invalid_argument(
            "--line-transition-replay cannot be combined with --line-transition-start or --line-transition-sessions",
        ));
    }
    if line_transition_replay_path.is_some()
        && (hot_args.iter().any(|argument| argument == "--workload")
            || hot_args
                .iter()
                .any(|argument| argument == "--write-workload"))
    {
        return Err(ToolError::invalid_argument(
            "--line-transition-replay supplies the hand-strategy workload and cannot be combined with --workload or --write-workload",
        ));
    }
    Ok(ThreeWayStabilityBenchmarkCommand {
        hot: parse_three_way_hot_benchmark_args(hot_args)?,
        runs,
        matrix_cache_capacities,
        matrix_cache_byte_budgets,
        line_transition_start,
        line_transition_sessions,
        line_transition_replay_path,
    })
}

fn parse_cache_byte_budgets(value: &str) -> Result<Vec<Option<usize>>, ToolError> {
    let mut budgets = Vec::new();
    for raw in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let normalized = raw.to_ascii_lowercase();
        let budget = match normalized.as_str() {
            "none" | "unbounded" => None,
            _ if normalized.ends_with("mib") => {
                let mib = normalized.trim_end_matches("mib").trim();
                let bytes = mib.parse::<usize>().map_err(|_| {
                    ToolError::invalid_argument(format!(
                        "--matrix-cache-byte-budgets contains invalid MiB value: {raw}"
                    ))
                })?;
                Some(bytes.checked_mul(1024 * 1024).ok_or_else(|| {
                    ToolError::invalid_argument(format!(
                        "--matrix-cache-byte-budgets overflows usize: {raw}"
                    ))
                })?)
            }
            _ => Some(normalized.parse::<usize>().map_err(|_| {
                ToolError::invalid_argument(format!(
                    "--matrix-cache-byte-budgets must use none, bytes, or MiB: {raw}"
                ))
            })?),
        };
        if !budgets.contains(&budget) {
            budgets.push(budget);
        }
    }
    if budgets.is_empty() {
        return Err(ToolError::invalid_argument(
            "--matrix-cache-byte-budgets must contain at least one value",
        ));
    }
    Ok(budgets)
}

pub fn run_three_way_stability_benchmark(
    command: &ThreeWayStabilityBenchmarkCommand,
) -> Result<ThreeWayStabilityBenchmarkReport, ToolError> {
    let replay_workload = command
        .line_transition_replay_path
        .as_ref()
        .map(|path| load_line_transition_replay(path))
        .transpose()?;
    let benchmark_workload = replay_workload
        .as_ref()
        .map(|workload| workload.as_benchmark_workload(command.hot.seed));
    let mut reports = Vec::with_capacity(command.runs);
    let mut raw_report_paths = Vec::with_capacity(command.runs);
    for run_index in 1..=command.runs {
        let mut run_command = command.hot.clone();
        run_command.out_path = per_run_path(&command.hot.out_path, run_index);
        run_command.md_path = per_run_path(&command.hot.md_path, run_index);
        let report = if let Some(workload) = &benchmark_workload {
            run_three_way_hot_benchmark_with_workload(
                &run_command,
                workload,
                crate::benchmark::types::WorkloadSource::Loaded,
                command
                    .line_transition_replay_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
            )?
        } else {
            run_three_way_hot_benchmark(&run_command)?
        };
        raw_report_paths.push(run_command.out_path.display().to_string());
        reports.push(report);
    }
    let profile_workload = benchmark_workload
        .as_ref()
        .cloned()
        .unwrap_or(load_workload(&command.hot)?);
    let matrix_cache_sweep = profile_matrix_cache_sweep(command, &profile_workload)?;
    let hand_strategy_profile = matrix_cache_sweep
        .first()
        .expect("cache sweep contains at least one configuration")
        .hand_strategy_profile
        .clone();
    let line_transition_sweep = profile_line_transition_sweep(command)?;
    let line_transition_replay_sweep = replay_workload
        .as_ref()
        .map(|workload| profile_line_transition_replay_sweep(command, workload))
        .transpose()?;
    let report = ThreeWayStabilityBenchmarkReport {
        generated_at: generated_at_utc(),
        runs: command.runs,
        raw_report_paths,
        cases: summarize_cases(&reports),
        metadata_cache: replay_workload
            .is_none()
            .then(|| measure_metadata_cache_phases(&command.hot))
            .transpose()?,
        hand_strategy_profile,
        matrix_cache_sweep,
        line_transition_sweep,
        line_transition_replay_sweep,
        notes: vec![
            "Development observation only: the underlying three-way runner validates result counts and does not use equivalent SQLite / Proto cache profiles; do not treat this summary as a formal performance or RSS baseline.".to_owned(),
            "Every run uses the same workload seed or supplied workload file; raw reports are retained beside this summary.".to_owned(),
            "Metadata phases use one Proto facade with max_open_handles=1: first query, cache hit, then optional query after LRU eviction when the workload includes drill metadata.".to_owned(),
            "Run-to-run statistics measure process and scheduling variation; they do not evict the OS page cache.".to_owned(),
        ],
    };
    write_json_report(&command.hot.out_path, &report)?;
    write_markdown_report(&command.hot.md_path, render_markdown(&report))?;
    Ok(report)
}

fn summarize_cases(reports: &[ThreeWayHotBenchmarkReport]) -> Vec<ThreeWayStabilityCase> {
    let Some(first_report) = reports.first() else {
        return Vec::new();
    };
    first_report
        .cases
        .iter()
        .map(|first_case| {
            let matching_cases = reports
                .iter()
                .map(|report| {
                    report
                        .cases
                        .iter()
                        .find(|case| case.name == first_case.name)
                        .expect("stability reports have identical cases")
                })
                .collect::<Vec<_>>();
            ThreeWayStabilityCase {
                name: first_case.name.clone(),
                proto_avg_ms: LatencySummary::from_values(
                    &matching_cases
                        .iter()
                        .map(|case| case.proto.avg_ms)
                        .collect::<Vec<_>>(),
                ),
                sqlite_avg_ms: LatencySummary::from_values(
                    &matching_cases
                        .iter()
                        .map(|case| case.sqlite.avg_ms)
                        .collect::<Vec<_>>(),
                ),
                proto_to_sqlite_avg_ratio: LatencySummary::from_values(
                    &matching_cases
                        .iter()
                        .map(|case| case.proto_to_sqlite_avg_latency_ratio)
                        .collect::<Vec<_>>(),
                ),
                core_avg_ms: LatencySummary::from_values(
                    &matching_cases
                        .iter()
                        .map(|case| case.core.avg_ms)
                        .collect::<Vec<_>>(),
                ),
            }
        })
        .collect()
}

fn measure_metadata_cache_phases(
    command: &ThreeWayHotBenchmarkCommand,
) -> Result<MetadataCachePhaseReport, ToolError> {
    let workload = load_workload(command)?;
    let target = workload
        .drill_scenario_queries
        .first()
        .cloned()
        .ok_or_else(|| {
            ToolError::invalid_format("stability workload has no drill metadata query")
        })?;
    let eviction_target = workload
        .drill_scenario_queries
        .iter()
        .find(|item| {
            item.strategy != target.strategy
                || item.player_count != target.player_count
                || item.drill_depth != target.drill_depth
        })
        .cloned();
    let facade = ProtoRangeStoreFacade::open(&command.proto_root, 1, command.verify_checksums)?;

    let before_first = get_memory_snapshot();
    let first_query_ms = measure_drill_query(&facade, &target)?;
    let after_first = get_memory_snapshot();
    let cache_hit_ms = measure_drill_query(&facade, &target)?;
    let after_hit = get_memory_snapshot();
    let (post_eviction_query_ms, after_eviction) = if let Some(eviction_target) = &eviction_target {
        measure_drill_query(&facade, eviction_target)?;
        let elapsed = measure_drill_query(&facade, &target)?;
        (Some(elapsed), Some(get_memory_snapshot()))
    } else {
        (None, None)
    };
    Ok(MetadataCachePhaseReport {
        target,
        eviction_target,
        first_query_ms,
        cache_hit_ms,
        post_eviction_query_ms,
        first_query_rss_delta_bytes: rss_delta(&before_first, &after_first),
        cache_hit_rss_delta_bytes: rss_delta(&after_first, &after_hit),
        post_eviction_rss_delta_bytes: after_eviction
            .as_ref()
            .and_then(|after| rss_delta(&after_hit, after)),
    })
}

fn profile_matrix_cache_sweep(
    command: &ThreeWayStabilityBenchmarkCommand,
    workload: &BenchmarkWorkload,
) -> Result<Vec<MatrixCacheSweepReport>, ToolError> {
    let mut reports = Vec::new();
    for &entry_capacity in &command.matrix_cache_capacities {
        for &byte_budget_bytes in &command.matrix_cache_byte_budgets {
            let config = MatrixCacheConfig {
                entry_capacity,
                byte_budget_bytes,
            };
            reports.push(MatrixCacheSweepReport {
                hand_strategy_profile: profile_hand_strategy_requests(
                    &command.hot,
                    &workload.hand_queries,
                    command.hot.warmup_iterations,
                    config.clone(),
                )?,
                config,
            });
        }
    }
    Ok(reports)
}

fn profile_hand_strategy_requests(
    command: &ThreeWayHotBenchmarkCommand,
    requests: &[HandBenchmarkItem],
    warmup_iterations: usize,
    cache_config: MatrixCacheConfig,
) -> Result<ProtoHandStrategyProfileReport, ToolError> {
    let facade = ProtoRangeStoreFacade::open_with_options(
        &command.proto_root,
        ProtoRangeStoreFacadeOptions {
            max_open_handles: command.max_open_handles,
            matrix_cache_capacity: cache_config.entry_capacity,
            matrix_cache_byte_budget: cache_config.byte_budget_bytes,
            verify_checksums: command.verify_checksums,
        },
    )?;
    for item in requests {
        facade.prewarm(&item.dimension())?;
    }
    let mut seen = HashMap::new();
    let mut access_history = Vec::new();
    for item in requests.iter().take(warmup_iterations) {
        let key = workload_matrix_key(item);
        record_workload_access(&mut seen, &mut access_history, key);
        facade.profile_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)?;
    }

    let mut facade_total = Vec::with_capacity(requests.len());
    let mut facade_overhead = Vec::with_capacity(requests.len());
    let mut dimension_check = Vec::with_capacity(requests.len());
    let mut parse_hand = Vec::with_capacity(requests.len());
    let mut matrix_read = Vec::with_capacity(requests.len());
    let mut matrix_cache_hits = 0usize;
    let mut matrix_first_seen_misses = 0usize;
    let mut matrix_revisit_after_eviction_misses = 0usize;
    let mut reuse_distances = Vec::new();
    let mut matrix_cache_evictions = 0u64;
    let mut matrix_cache_oversized_skips = 0u64;
    let mut last_cache_counters = HashMap::<String, (u64, u64)>::new();
    let mut max_observed_resident_estimated_bytes = 0usize;
    let mut max_observed_peak_resident_estimated_bytes = 0usize;
    let handle_stats_before = facade.handle_pool_stats();
    let mut matrix_index_payload = Vec::new();
    let mut matrix_protobuf_decode = Vec::new();
    let mut matrix_compact_index = Vec::new();
    let mut matrix_cache_insert = Vec::new();
    let mut action_materialization = Vec::with_capacity(requests.len());
    let mut service_total = Vec::with_capacity(requests.len());
    let mut slowest = Vec::with_capacity(requests.len());
    for item in requests {
        let key = workload_matrix_key(item);
        let is_first_seen = !seen.contains_key(&key);
        if let Some(distance) = reuse_distance(&access_history, &seen, &key) {
            reuse_distances.push(distance as f64);
        }
        let profiled = facade.profile_hand_strategy(
            &item.dimension(),
            item.concrete_line_id,
            &item.hole_cards,
        )?;
        let phases = &profiled.profiled.profile;
        facade_total.push(profiled.facade_total_ms);
        facade_overhead.push((profiled.facade_total_ms - phases.service_total_ms).max(0.0));
        dimension_check.push(phases.dimension_check_ms);
        parse_hand.push(phases.parse_hand_ms);
        matrix_read.push(phases.matrix_read_ms);
        if phases.matrix_cache_hit {
            matrix_cache_hits += 1;
        } else {
            if is_first_seen {
                matrix_first_seen_misses += 1;
            } else {
                matrix_revisit_after_eviction_misses += 1;
            }
            matrix_index_payload.push(phases.matrix_index_payload_ms);
            matrix_protobuf_decode.push(phases.matrix_protobuf_decode_ms);
            matrix_compact_index.push(phases.matrix_compact_index_ms);
            matrix_cache_insert.push(phases.matrix_cache_insert_ms);
        }
        let cache_stats = facade.matrix_cache_stats(&item.dimension())?;
        let dimension_key = format!(
            "{}:{}max:{}BB",
            item.strategy, item.player_count, item.depth_bb
        );
        let (previous_evictions, previous_oversized_skips) = last_cache_counters
            .insert(
                dimension_key,
                (cache_stats.evictions, cache_stats.oversized_skips),
            )
            .unwrap_or((0, 0));
        matrix_cache_evictions = matrix_cache_evictions
            .wrapping_add(counter_delta(cache_stats.evictions, previous_evictions));
        matrix_cache_oversized_skips = matrix_cache_oversized_skips.wrapping_add(counter_delta(
            cache_stats.oversized_skips,
            previous_oversized_skips,
        ));
        max_observed_resident_estimated_bytes =
            max_observed_resident_estimated_bytes.max(cache_stats.resident_estimated_bytes);
        max_observed_peak_resident_estimated_bytes = max_observed_peak_resident_estimated_bytes
            .max(cache_stats.peak_resident_estimated_bytes);
        action_materialization.push(phases.action_materialization_ms);
        service_total.push(phases.service_total_ms);
        slowest.push(SlowHandStrategyProfile {
            dimension: format!(
                "{}:{}max:{}BB",
                item.strategy, item.player_count, item.depth_bb
            ),
            concrete_line_id: item.concrete_line_id,
            hand: item.hole_cards.clone(),
            facade_total_ms: profiled.facade_total_ms,
            matrix_read_ms: phases.matrix_read_ms,
            action_materialization_ms: phases.action_materialization_ms,
        });
        record_workload_access(&mut seen, &mut access_history, key);
    }
    let handle_stats_after = facade.handle_pool_stats();
    slowest.sort_by(|left, right| right.facade_total_ms.total_cmp(&left.facade_total_ms));
    slowest.truncate(10);
    Ok(ProtoHandStrategyProfileReport {
        cache_config,
        warmup_requests: warmup_iterations.min(requests.len()),
        samples: facade_total.len(),
        facade_total_ms: LatencySummary::from_values(&facade_total),
        facade_overhead_ms: LatencySummary::from_values(&facade_overhead),
        dimension_check_ms: LatencySummary::from_values(&dimension_check),
        parse_hand_ms: LatencySummary::from_values(&parse_hand),
        matrix_read_ms: LatencySummary::from_values(&matrix_read),
        matrix_cache_hits,
        matrix_cache_misses: facade_total.len() - matrix_cache_hits,
        matrix_first_seen_misses,
        matrix_revisit_after_eviction_misses,
        unique_matrix_count: seen.len(),
        reuse_distance: (!reuse_distances.is_empty())
            .then(|| LatencySummary::from_values(&reuse_distances)),
        matrix_cache_evictions,
        matrix_cache_oversized_skips,
        max_observed_resident_estimated_bytes,
        max_observed_peak_resident_estimated_bytes,
        dimension_handle_opens: handle_stats_after.opens - handle_stats_before.opens,
        dimension_handle_evictions: handle_stats_after.evictions - handle_stats_before.evictions,
        matrix_index_payload_ms: LatencySummary::from_values(&matrix_index_payload),
        matrix_protobuf_decode_ms: LatencySummary::from_values(&matrix_protobuf_decode),
        matrix_compact_index_ms: LatencySummary::from_values(&matrix_compact_index),
        matrix_cache_insert_ms: LatencySummary::from_values(&matrix_cache_insert),
        action_materialization_ms: LatencySummary::from_values(&action_materialization),
        service_total_ms: LatencySummary::from_values(&service_total),
        slowest,
    })
}

fn profile_line_transition_sweep(
    command: &ThreeWayStabilityBenchmarkCommand,
) -> Result<Option<LineTransitionSweepReport>, ToolError> {
    if command.line_transition_sessions == 0 {
        return Ok(None);
    }
    let start_concrete_line = command
        .line_transition_start
        .as_deref()
        .expect("validated with line transition sessions");
    if command.hot.requested_dimensions.len() != 1 {
        return Err(ToolError::invalid_argument(
            "line-transition observation requires exactly one --dimension",
        ));
    }
    let workload = build_line_transition_workload(
        &command.hot.source_db,
        command.hot.requested_dimensions[0].clone(),
        start_concrete_line,
        command.line_transition_sessions,
    )?;
    let mut cache_configs = Vec::new();
    for &entry_capacity in &command.matrix_cache_capacities {
        for &byte_budget_bytes in &command.matrix_cache_byte_budgets {
            let config = MatrixCacheConfig {
                entry_capacity,
                byte_budget_bytes,
            };
            cache_configs.push(MatrixCacheSweepReport {
                hand_strategy_profile: profile_hand_strategy_requests(
                    &command.hot,
                    &workload.requests,
                    0,
                    config.clone(),
                )?,
                config,
            });
        }
    }
    Ok(Some(LineTransitionSweepReport {
        dimension: format!(
            "{}:{}max:{}BB",
            workload.dimension.strategy,
            workload.dimension.player_count,
            workload.dimension.depth_bb
        ),
        start_concrete_line: workload.start_concrete_line,
        sessions: workload.sessions,
        steps: workload.requests.len(),
        candidate_leaf_count: workload.candidate_leaf_count,
        implicit_fold_normalized_prefix_count: workload.implicit_fold_normalized_prefix_count,
        skipped_unresolvable_leaf_count: workload.skipped_unresolvable_leaf_count,
        skipped_no_retained_hand_leaf_count: workload.skipped_no_retained_hand_leaf_count,
        child_fanout: (!workload.child_fanout.is_empty())
            .then(|| LatencySummary::from_values(&workload.child_fanout)),
        cache_configs,
    }))
}

fn profile_line_transition_replay_sweep(
    command: &ThreeWayStabilityBenchmarkCommand,
    workload: &LineTransitionReplayWorkload,
) -> Result<LineTransitionReplaySweepReport, ToolError> {
    let mut cache_configs = Vec::new();
    for &entry_capacity in &command.matrix_cache_capacities {
        for &byte_budget_bytes in &command.matrix_cache_byte_budgets {
            let config = MatrixCacheConfig {
                entry_capacity,
                byte_budget_bytes,
            };
            let hand_strategy_profile = profile_hand_strategy_requests(
                &command.hot,
                &workload.requests,
                0,
                config.clone(),
            )?;
            let session_total_ms = measure_replay_session_totals(&command.hot, workload, &config)?;
            cache_configs.push(LineTransitionReplayCacheConfigReport {
                config,
                session_total_ms,
                hand_strategy_profile,
            });
        }
    }
    Ok(LineTransitionReplaySweepReport {
        replay_path: workload.replay_path.display().to_string(),
        sessions: workload.sessions.len(),
        steps: workload.requests.len(),
        dimensions: workload.dimensions.clone(),
        cache_configs,
    })
}

fn measure_replay_session_totals(
    command: &ThreeWayHotBenchmarkCommand,
    workload: &LineTransitionReplayWorkload,
    config: &MatrixCacheConfig,
) -> Result<LatencySummary, ToolError> {
    let facade = ProtoRangeStoreFacade::open_with_options(
        &command.proto_root,
        ProtoRangeStoreFacadeOptions {
            max_open_handles: command.max_open_handles,
            matrix_cache_capacity: config.entry_capacity,
            matrix_cache_byte_budget: config.byte_budget_bytes,
            verify_checksums: command.verify_checksums,
        },
    )?;
    let mut totals = Vec::with_capacity(workload.sessions.len());
    for session in &workload.sessions {
        let started = Instant::now();
        for request in &session.requests {
            facade.query_hand_strategy(
                &request.dimension(),
                request.concrete_line_id,
                &request.hole_cards,
            )?;
        }
        totals.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(LatencySummary::from_values(&totals))
}

fn load_line_transition_replay(path: &Path) -> Result<LineTransitionReplayWorkload, ToolError> {
    let bytes = fs::read(path)?;
    let replay: CanonicalLineReplay = serde_json::from_slice(&bytes).map_err(|error| {
        ToolError::invalid_format(format!("invalid canonical replay JSON: {error}"))
    })?;
    if replay.schema_version != 1 {
        return Err(ToolError::invalid_format(format!(
            "unsupported canonical replay schemaVersion {} (expected 1)",
            replay.schema_version
        )));
    }
    if replay.sessions.is_empty() {
        return Err(ToolError::invalid_format(
            "canonical replay must contain at least one session",
        ));
    }

    let mut requests = Vec::new();
    let mut dimensions = Vec::new();
    for (session_index, session) in replay.sessions.iter().enumerate() {
        if session.name.trim().is_empty() {
            return Err(ToolError::invalid_format(format!(
                "canonical replay session {session_index} has an empty name"
            )));
        }
        if session.requests.is_empty() {
            return Err(ToolError::invalid_format(format!(
                "canonical replay session {} has no requests",
                session.name
            )));
        }
        for request in &session.requests {
            let dimension = format!(
                "{}:{}max:{}BB",
                request.strategy, request.player_count, request.depth_bb
            );
            if !dimensions.contains(&dimension) {
                dimensions.push(dimension);
            }
            requests.push(request.clone());
        }
    }
    let workload = LineTransitionReplayWorkload {
        replay_path: path.to_owned(),
        sessions: replay.sessions,
        requests,
        dimensions,
    };
    Ok(workload)
}

fn build_line_transition_workload(
    source_db: &Path,
    dimension: DimensionRef,
    start_concrete_line: &str,
    sessions: usize,
) -> Result<LineTransitionWorkload, ToolError> {
    let connection = Connection::open(source_db, true)?;
    let concrete_table = quote_identifier(&get_concrete_lines_table_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    ))
    .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
    let mut line_statement = connection.prepare(&format!(
        "SELECT id, concrete_line FROM {concrete_table} ORDER BY concrete_line"
    ))?;
    line_statement.start(&[])?;
    let mut lines = Vec::new();
    while line_statement.step_row()? {
        lines.push(LineTransitionSourceLine {
            concrete_line_id: line_statement.column_u32(0)?,
            concrete_line: line_statement.column_text(1)?,
        });
    }
    let line_ids = lines
        .iter()
        .map(|line| (line.concrete_line.clone(), line.concrete_line_id))
        .collect::<HashMap<_, _>>();
    if !line_ids.contains_key(start_concrete_line) {
        return Err(ToolError::invalid_argument(format!(
            "line-transition start concrete line does not exist: {start_concrete_line}"
        )));
    }

    let range_table = quote_identifier(&format!(
        "range_data_{}_{}max_{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    ))
    .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
    let mut hand_statement = connection.prepare(&format!(
        "SELECT concrete_line_id, MIN(hole_cards) FROM {range_table} \
         WHERE hand_ev IS NOT NULL GROUP BY concrete_line_id"
    ))?;
    hand_statement.start(&[])?;
    let mut hands = HashMap::new();
    while hand_statement.step_row()? {
        hands.insert(
            hand_statement.column_u32(0)?,
            hand_statement.column_text(1)?,
        );
    }

    let mut parents = HashSet::new();
    for line in line_ids.keys() {
        if let Some(parent) = concrete_line_parent(line) {
            parents.insert(parent.to_owned());
        }
    }
    let mut leaves = line_ids
        .keys()
        .filter(|line| is_line_descendant(line, start_concrete_line) && !parents.contains(*line))
        .cloned()
        .collect::<Vec<_>>();
    leaves.sort();
    if leaves.is_empty() {
        return Err(ToolError::invalid_format(format!(
            "line-transition start has no leaf descendants: {start_concrete_line}"
        )));
    }

    let candidate_leaf_count = leaves.len();
    let mut complete_paths = Vec::new();
    let mut child_edges = HashMap::<String, HashSet<String>>::new();
    let mut implicit_fold_normalized_prefix_count = 0;
    let mut skipped_unresolvable_leaf_count = 0;
    let mut skipped_no_retained_hand_leaf_count = 0;
    for leaf in leaves {
        let Ok((path, normalized_prefix_count)) = canonical_line_transition_path(
            &leaf,
            start_concrete_line,
            &line_ids,
            dimension.player_count as usize,
        ) else {
            skipped_unresolvable_leaf_count += 1;
            continue;
        };
        let mut steps = Vec::with_capacity(path.len());
        let mut has_retained_hand_for_every_step = true;
        for concrete_line in path {
            let Some(&concrete_line_id) = line_ids.get(&concrete_line) else {
                skipped_unresolvable_leaf_count += 1;
                has_retained_hand_for_every_step = false;
                break;
            };
            let Some(hole_cards) = hands.get(&concrete_line_id) else {
                skipped_no_retained_hand_leaf_count += 1;
                has_retained_hand_for_every_step = false;
                break;
            };
            steps.push((concrete_line, concrete_line_id, hole_cards.clone()));
        }
        if !has_retained_hand_for_every_step {
            continue;
        }
        implicit_fold_normalized_prefix_count += normalized_prefix_count;
        for edge in steps.windows(2) {
            child_edges
                .entry(edge[0].0.clone())
                .or_default()
                .insert(edge[1].0.clone());
        }
        complete_paths.push(steps);
    }
    if complete_paths.is_empty() {
        return Err(ToolError::invalid_format(format!(
            "line-transition start has no resolvable retained paths: {start_concrete_line}"
        )));
    }

    let mut requests = Vec::new();
    let mut observed_child_fanout = Vec::new();
    for session_index in 0..sessions {
        let path_index = if sessions <= complete_paths.len() {
            session_index * complete_paths.len() / sessions
        } else {
            session_index % complete_paths.len()
        };
        let path = &complete_paths[path_index];
        for (step_index, (concrete_line, concrete_line_id, hole_cards)) in path.iter().enumerate() {
            requests.push(HandBenchmarkItem {
                strategy: dimension.strategy.clone(),
                player_count: dimension.player_count,
                depth_bb: dimension.depth_bb,
                concrete_line_id: *concrete_line_id,
                hole_cards: hole_cards.clone(),
            });
            if step_index + 1 < path.len() {
                observed_child_fanout.push(
                    child_edges
                        .get(concrete_line)
                        .expect("canonical path parent has a selected child")
                        .len() as f64,
                );
            }
        }
    }
    Ok(LineTransitionWorkload {
        dimension,
        start_concrete_line: start_concrete_line.to_owned(),
        sessions,
        requests,
        child_fanout: observed_child_fanout,
        candidate_leaf_count,
        implicit_fold_normalized_prefix_count,
        skipped_unresolvable_leaf_count,
        skipped_no_retained_hand_leaf_count,
    })
}

fn concrete_line_parent(line: &str) -> Option<&str> {
    if line.is_empty() {
        None
    } else {
        Some(line.rsplit_once('-').map_or("", |(parent, _)| parent))
    }
}

fn is_line_descendant(line: &str, start: &str) -> bool {
    start.is_empty() || line == start || line.starts_with(&format!("{start}-"))
}

fn canonical_line_transition_path(
    leaf: &str,
    start: &str,
    line_ids: &HashMap<String, u32>,
    player_count: usize,
) -> Result<(Vec<String>, usize), ToolError> {
    if !is_line_descendant(leaf, start) {
        return Err(ToolError::invalid_format(format!(
            "line-transition leaf {leaf} is not a descendant of start {start}"
        )));
    }

    let start_token_count = concrete_line_token_count(start);
    let leaf_tokens = concrete_line_tokens(leaf);
    let mut path = Vec::new();
    let mut normalized_prefix_count = 0;
    for token_count in start_token_count..=leaf_tokens.len() {
        let raw_prefix = concrete_line_from_tokens(&leaf_tokens[..token_count]);
        let canonical_line = resolve_implicit_fold_line(&raw_prefix, line_ids, player_count)
            .ok_or_else(|| {
                ToolError::invalid_format(format!(
                    "line-transition prefix has no stored canonical matrix: {raw_prefix}"
                ))
            })?;
        if canonical_line != raw_prefix {
            normalized_prefix_count += 1;
        }
        if path.last() != Some(&canonical_line) {
            path.push(canonical_line);
        }
    }
    Ok((path, normalized_prefix_count))
}

fn resolve_implicit_fold_line(
    raw_prefix: &str,
    line_ids: &HashMap<String, u32>,
    player_count: usize,
) -> Option<String> {
    if line_ids.contains_key(raw_prefix) {
        return Some(raw_prefix.to_owned());
    }

    let mut canonical_line = raw_prefix.to_owned();
    for _ in 0..player_count {
        if canonical_line.is_empty() {
            canonical_line.push('F');
        } else {
            canonical_line.push_str("-F");
        }
        if line_ids.contains_key(&canonical_line) {
            return Some(canonical_line);
        }
    }
    None
}

fn concrete_line_tokens(line: &str) -> Vec<&str> {
    if line.is_empty() {
        Vec::new()
    } else {
        line.split('-').collect()
    }
}

fn concrete_line_token_count(line: &str) -> usize {
    if line.is_empty() {
        0
    } else {
        line.split('-').count()
    }
}

fn concrete_line_from_tokens(tokens: &[&str]) -> String {
    tokens.join("-")
}

fn load_workload(command: &ThreeWayHotBenchmarkCommand) -> Result<BenchmarkWorkload, ToolError> {
    if let Some(path) = &command.workload_path {
        return read_workload_json(path);
    }
    create_benchmark_workload(&WorkloadOptions {
        source_db_path: command.source_db.clone(),
        requested_dimensions: command.requested_dimensions.clone(),
        seed: command.seed,
        hand_iterations: command.hand_iterations,
        batch_iterations: command.batch_iterations,
        batch_size: command.batch_size,
        batch_sizes: command.batch_sizes.clone(),
        workload_mode: command.workload_mode,
    })
}

fn workload_matrix_key(item: &HandBenchmarkItem) -> WorkloadMatrixKey {
    WorkloadMatrixKey {
        dimension: format!(
            "{}:{}max:{}BB",
            item.strategy, item.player_count, item.depth_bb
        ),
        concrete_line_id: item.concrete_line_id,
    }
}

fn record_workload_access(
    seen: &mut HashMap<WorkloadMatrixKey, usize>,
    history: &mut Vec<WorkloadMatrixKey>,
    key: WorkloadMatrixKey,
) {
    let index = history.len();
    seen.insert(key.clone(), index);
    history.push(key);
}

fn reuse_distance(
    history: &[WorkloadMatrixKey],
    seen: &HashMap<WorkloadMatrixKey, usize>,
    key: &WorkloadMatrixKey,
) -> Option<usize> {
    let last_index = *seen.get(key)?;
    Some(
        history[last_index.saturating_add(1)..]
            .iter()
            .collect::<HashSet<_>>()
            .len(),
    )
}

fn counter_delta(current: u64, previous: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn measure_drill_query(
    facade: &ProtoRangeStoreFacade,
    item: &DrillScenarioBenchmarkItem,
) -> Result<f64, ToolError> {
    let started = Instant::now();
    facade.get_drill_scenario_lines(
        &item.strategy,
        &item.drill_name,
        item.player_count,
        item.drill_depth,
    )?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

fn rss_delta(before: &MemorySnapshot, after: &MemorySnapshot) -> Option<i64> {
    Some(after.rss_bytes? as i64 - before.rss_bytes? as i64)
}

fn per_run_path(path: &Path, run_index: usize) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("json");
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("benchmark");
    path.with_file_name(format!("{stem}.run-{run_index}.{extension}"))
}

fn render_markdown(report: &ThreeWayStabilityBenchmarkReport) -> String {
    let mut markdown = String::from(
        "# Core vs Proto vs SQLite Stability Development Observation (Not a Formal Baseline)\n\n",
    );
    markdown.push_str(&format!("- Repetitions: `{}`\n\n", report.runs));
    markdown.push_str(
        "> Underlying runs validate result counts only and do not use equivalent SQLite / Proto cache profiles. The ratios below are not formal performance or RSS evidence.\n\n",
    );
    let rows = report
        .cases
        .iter()
        .map(|case| {
            vec![
                case.name.clone(),
                format!("{:.4}", case.proto_avg_ms.p50_ms),
                format!("{:.4}", case.proto_avg_ms.p95_ms),
                format!("{:.4}", case.sqlite_avg_ms.p50_ms),
                format!("{:.4}", case.sqlite_avg_ms.p95_ms),
                format!("{:.2}x", case.proto_to_sqlite_avg_ratio.p50_ms),
                format!("{:.2}x", case.proto_to_sqlite_avg_ratio.p95_ms),
            ]
        })
        .collect::<Vec<_>>();
    markdown.push_str("## Cross-Run P50/P95\n\n");
    markdown.push_str(&markdown_table(
        &[
            "case",
            "Proto P50 ms",
            "Proto P95 ms",
            "SQLite P50 ms",
            "SQLite P95 ms",
            "P/S P50",
            "P/S P95",
        ],
        &rows,
    ));
    if let Some(metadata_cache) = &report.metadata_cache {
        markdown.push_str("\n## Proto Metadata Cache Phases\n\n");
        markdown.push_str(&markdown_table(
            &[
                "first ms",
                "cache hit ms",
                "post-eviction ms",
                "first RSS delta",
                "cache-hit RSS delta",
                "post-eviction RSS delta",
            ],
            &[vec![
                format!("{:.4}", metadata_cache.first_query_ms),
                format!("{:.4}", metadata_cache.cache_hit_ms),
                metadata_cache
                    .post_eviction_query_ms
                    .map(|value| format!("{value:.4}"))
                    .unwrap_or_else(|| "n/a".to_owned()),
                format_optional_bytes(metadata_cache.first_query_rss_delta_bytes),
                format_optional_bytes(metadata_cache.cache_hit_rss_delta_bytes),
                format_optional_bytes(metadata_cache.post_eviction_rss_delta_bytes),
            ]],
        ));
    }
    let profile = &report.hand_strategy_profile;
    markdown.push_str("\n## Proto Hand Strategy Phase Profile\n\n");
    markdown.push_str(&markdown_table(
        &[
            "samples",
            "facade P50/P95",
            "matrix read P50/P95",
            "action materialization P50/P95",
            "facade overhead P50/P95",
        ],
        &[vec![
            profile.samples.to_string(),
            format!(
                "{:.4}/{:.4}",
                profile.facade_total_ms.p50_ms, profile.facade_total_ms.p95_ms
            ),
            format!(
                "{:.4}/{:.4}",
                profile.matrix_read_ms.p50_ms, profile.matrix_read_ms.p95_ms
            ),
            format!(
                "{:.4}/{:.4}",
                profile.action_materialization_ms.p50_ms, profile.action_materialization_ms.p95_ms
            ),
            format!(
                "{:.4}/{:.4}",
                profile.facade_overhead_ms.p50_ms, profile.facade_overhead_ms.p95_ms
            ),
        ]],
    ));
    markdown.push_str(&format!(
        "\nMatrix cache: {} hits, {} misses. Miss phase P50: payload {:.4} ms, Protobuf decode {:.4} ms, compact index {:.4} ms, cache insert {:.4} ms.\n",
        profile.matrix_cache_hits,
        profile.matrix_cache_misses,
        profile.matrix_index_payload_ms.p50_ms,
        profile.matrix_protobuf_decode_ms.p50_ms,
        profile.matrix_compact_index_ms.p50_ms,
        profile.matrix_cache_insert_ms.p50_ms,
    ));
    markdown.push_str("\n### Cache Locality\n\n");
    markdown.push_str(&markdown_table(
        &[
            "entry capacity",
            "byte budget",
            "warmup requests",
            "unique matrices",
            "first-seen misses",
            "revisit misses",
            "LRU evictions",
            "oversized skips",
            "handle opens/evictions",
            "reuse distance P50/P95",
            "max resident/peak estimate",
        ],
        &[vec![
            profile.cache_config.entry_capacity.to_string(),
            format_optional_usize(profile.cache_config.byte_budget_bytes),
            profile.warmup_requests.to_string(),
            profile.unique_matrix_count.to_string(),
            profile.matrix_first_seen_misses.to_string(),
            profile.matrix_revisit_after_eviction_misses.to_string(),
            profile.matrix_cache_evictions.to_string(),
            profile.matrix_cache_oversized_skips.to_string(),
            format!(
                "{}/{}",
                profile.dimension_handle_opens, profile.dimension_handle_evictions
            ),
            format_optional_latency(profile.reuse_distance.as_ref()),
            format!(
                "{}/{} B",
                profile.max_observed_resident_estimated_bytes,
                profile.max_observed_peak_resident_estimated_bytes
            ),
        ]],
    ));
    markdown.push_str("\n## Matrix Cache Sweep\n\n");
    let sweep_rows = report
        .matrix_cache_sweep
        .iter()
        .map(|item| {
            let profile = &item.hand_strategy_profile;
            vec![
                item.config.entry_capacity.to_string(),
                format_optional_usize(item.config.byte_budget_bytes),
                format!(
                    "{}/{}",
                    profile.matrix_cache_hits, profile.matrix_cache_misses
                ),
                format!(
                    "{}/{}",
                    profile.matrix_first_seen_misses, profile.matrix_revisit_after_eviction_misses
                ),
                profile.matrix_cache_evictions.to_string(),
                format!(
                    "{:.4}/{:.4}",
                    profile.matrix_read_ms.p50_ms, profile.matrix_read_ms.p95_ms
                ),
                format!(
                    "{:.4}/{:.4}",
                    profile.facade_total_ms.p50_ms, profile.facade_total_ms.p95_ms
                ),
                profile
                    .max_observed_peak_resident_estimated_bytes
                    .to_string(),
            ]
        })
        .collect::<Vec<_>>();
    markdown.push_str(&markdown_table(
        &[
            "entry capacity",
            "byte budget",
            "hits/misses",
            "first/revisit misses",
            "LRU evictions",
            "matrix P50/P95 ms",
            "facade P50/P95 ms",
            "peak estimate B",
        ],
        &sweep_rows,
    ));
    if let Some(transition) = &report.line_transition_sweep {
        markdown.push_str("\n## Line Transition Session Sweep\n\n");
        markdown.push_str(&markdown_table(
            &[
                "dimension",
                "start line",
                "sessions",
                "steps",
                "candidate leaves",
                "implicit F prefixes",
                "unresolvable/no-hand leaves",
                "child fanout P50/P95",
            ],
            &[vec![
                transition.dimension.clone(),
                transition.start_concrete_line.clone(),
                transition.sessions.to_string(),
                transition.steps.to_string(),
                transition.candidate_leaf_count.to_string(),
                transition.implicit_fold_normalized_prefix_count.to_string(),
                format!(
                    "{}/{}",
                    transition.skipped_unresolvable_leaf_count,
                    transition.skipped_no_retained_hand_leaf_count
                ),
                format_optional_latency(transition.child_fanout.as_ref()),
            ]],
        ));
        let transition_rows = transition
            .cache_configs
            .iter()
            .map(|item| {
                let profile = &item.hand_strategy_profile;
                vec![
                    item.config.entry_capacity.to_string(),
                    format_optional_usize(item.config.byte_budget_bytes),
                    format!(
                        "{}/{}",
                        profile.matrix_cache_hits, profile.matrix_cache_misses
                    ),
                    format!(
                        "{}/{}",
                        profile.matrix_first_seen_misses,
                        profile.matrix_revisit_after_eviction_misses
                    ),
                    profile.matrix_cache_evictions.to_string(),
                    format!(
                        "{:.4}/{:.4}",
                        profile.matrix_read_ms.p50_ms, profile.matrix_read_ms.p95_ms
                    ),
                    profile
                        .max_observed_peak_resident_estimated_bytes
                        .to_string(),
                ]
            })
            .collect::<Vec<_>>();
        markdown.push_str("\nChild prewarm is not enabled in this measurement; this is the natural parent-to-child traversal baseline.\n\n");
        markdown.push_str(&markdown_table(
            &[
                "entry capacity",
                "byte budget",
                "hits/misses",
                "first/revisit misses",
                "LRU evictions",
                "matrix P50/P95 ms",
                "peak estimate B",
            ],
            &transition_rows,
        ));
    }
    if let Some(replay) = &report.line_transition_replay_sweep {
        markdown.push_str("\n## Canonical Replay Session Sweep\n\n");
        markdown.push_str(&markdown_table(
            &["replay", "sessions", "steps", "dimensions"],
            &[vec![
                replay.replay_path.clone(),
                replay.sessions.to_string(),
                replay.steps.to_string(),
                replay.dimensions.join(", "),
            ]],
        ));
        let replay_rows = replay
            .cache_configs
            .iter()
            .map(|item| {
                let profile = &item.hand_strategy_profile;
                vec![
                    item.config.entry_capacity.to_string(),
                    format_optional_usize(item.config.byte_budget_bytes),
                    format!(
                        "{:.4}/{:.4}",
                        item.session_total_ms.p50_ms, item.session_total_ms.p95_ms
                    ),
                    format!(
                        "{}/{}",
                        profile.matrix_cache_hits, profile.matrix_cache_misses
                    ),
                    format!(
                        "{}/{}",
                        profile.matrix_first_seen_misses,
                        profile.matrix_revisit_after_eviction_misses
                    ),
                    profile
                        .max_observed_peak_resident_estimated_bytes
                        .to_string(),
                ]
            })
            .collect::<Vec<_>>();
        markdown.push_str(&markdown_table(
            &[
                "entry capacity",
                "byte budget",
                "session total P50/P95 ms",
                "hits/misses",
                "first/revisit misses",
                "peak estimate B",
            ],
            &replay_rows,
        ));
    }
    let slow_rows = profile
        .slowest
        .iter()
        .map(|item| {
            vec![
                item.dimension.clone(),
                item.concrete_line_id.to_string(),
                item.hand.clone(),
                format!("{:.4}", item.facade_total_ms),
                format!("{:.4}", item.matrix_read_ms),
                format!("{:.4}", item.action_materialization_ms),
            ]
        })
        .collect::<Vec<_>>();
    markdown.push_str("\n### Slowest Requests\n\n");
    markdown.push_str(&markdown_table(
        &[
            "dimension",
            "line",
            "hand",
            "facade ms",
            "matrix read ms",
            "action materialization ms",
        ],
        &slow_rows,
    ));
    markdown.push_str("\n## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn format_optional_bytes(value: Option<i64>) -> String {
    value
        .map(|value| format!("{value} B"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_usize(value: Option<usize>) -> String {
    value
        .map(|value| format!("{value} B"))
        .unwrap_or_else(|| "unbounded".to_owned())
}

fn format_optional_latency(value: Option<&LatencySummary>) -> String {
    value
        .map(|value| format!("{:.1}/{:.1}", value.p50_ms, value.p95_ms))
        .unwrap_or_else(|| "n/a".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cache_byte_budgets_with_unbounded_and_mib_values() {
        assert_eq!(
            parse_cache_byte_budgets("none,16MiB,1024,unbounded").expect("parse budgets"),
            vec![None, Some(16 * 1024 * 1024), Some(1024)]
        );
    }

    #[test]
    fn parses_matrix_cache_sweep_options_without_forwarding_them_to_hot_benchmark() {
        let command = parse_three_way_stability_benchmark_args(vec![
            "--source".to_owned(),
            "source.db".to_owned(),
            "--proto-root".to_owned(),
            "proto".to_owned(),
            "--core-dir".to_owned(),
            "core".to_owned(),
            "--runs".to_owned(),
            "2".to_owned(),
            "--matrix-cache-capacities".to_owned(),
            "128,1024".to_owned(),
            "--matrix-cache-byte-budgets".to_owned(),
            "none,16MiB".to_owned(),
        ])
        .expect("parse stability command");

        assert_eq!(command.matrix_cache_capacities, vec![128, 1024]);
        assert_eq!(
            command.matrix_cache_byte_budgets,
            vec![None, Some(16 * 1024 * 1024)]
        );
    }

    #[test]
    fn parses_line_transition_options() {
        let command = parse_three_way_stability_benchmark_args(vec![
            "--source".to_owned(),
            "source.db".to_owned(),
            "--proto-root".to_owned(),
            "proto".to_owned(),
            "--core-dir".to_owned(),
            "core".to_owned(),
            "--dimension".to_owned(),
            "default:6:100".to_owned(),
            "--line-transition-start".to_owned(),
            "F-F".to_owned(),
            "--line-transition-sessions".to_owned(),
            "20".to_owned(),
        ])
        .expect("parse line transition command");

        assert_eq!(command.line_transition_start.as_deref(), Some("F-F"));
        assert_eq!(command.line_transition_sessions, 20);
    }

    #[test]
    fn parses_canonical_line_transition_replay_option() {
        let command = parse_three_way_stability_benchmark_args(vec![
            "--source".to_owned(),
            "source.db".to_owned(),
            "--proto-root".to_owned(),
            "proto".to_owned(),
            "--core-dir".to_owned(),
            "core".to_owned(),
            "--line-transition-replay".to_owned(),
            "replay.json".to_owned(),
        ])
        .expect("parse canonical replay command");

        assert_eq!(
            command.line_transition_replay_path,
            Some(PathBuf::from("replay.json"))
        );
    }

    #[test]
    fn canonical_replay_converts_only_ordered_hand_strategy_requests() {
        let replay = LineTransitionReplayWorkload {
            replay_path: PathBuf::from("replay.json"),
            sessions: vec![CanonicalLineReplaySession {
                name: "three-bet-branch".to_owned(),
                requests: vec![HandBenchmarkItem {
                    strategy: "default".to_owned(),
                    player_count: 6,
                    depth_bb: 100,
                    concrete_line_id: 42,
                    hole_cards: "AKs".to_owned(),
                }],
            }],
            requests: vec![HandBenchmarkItem {
                strategy: "default".to_owned(),
                player_count: 6,
                depth_bb: 100,
                concrete_line_id: 42,
                hole_cards: "AKs".to_owned(),
            }],
            dimensions: vec!["default:6max:100BB".to_owned()],
        };

        let workload = replay.as_benchmark_workload(7);
        assert_eq!(workload.seed, 7);
        assert_eq!(workload.hand_queries, replay.requests);
        assert!(workload.batch_queries.is_empty());
        assert!(workload.hands_by_actions_queries.is_empty());
        assert!(workload.drill_scenario_queries.is_empty());
    }

    #[test]
    fn builds_prefix_path_from_start_to_leaf() {
        let line_ids = test_line_ids(&["F-F", "F-F-R2", "F-F-R2-C", "F-F-R2-C-F"]);
        assert_eq!(
            canonical_line_transition_path("F-F-R2-C-F", "F-F", &line_ids, 6)
                .expect("transition path")
                .0,
            vec![
                "F-F".to_owned(),
                "F-F-R2".to_owned(),
                "F-F-R2-C".to_owned(),
                "F-F-R2-C-F".to_owned(),
            ]
        );
    }

    #[test]
    fn treats_the_empty_concrete_line_as_the_root_matrix() {
        assert_eq!(concrete_line_parent("F"), Some(""));
        let line_ids = test_line_ids(&["", "F", "F-F", "F-F-R2"]);
        assert_eq!(
            canonical_line_transition_path("F-F-R2", "", &line_ids, 6)
                .expect("root transition path")
                .0,
            vec![
                "".to_owned(),
                "F".to_owned(),
                "F-F".to_owned(),
                "F-F-R2".to_owned(),
            ]
        );
        assert!(is_line_descendant("F-F-R2", ""));
        assert!(!is_line_descendant("F-R2", "F-F"));
    }

    #[test]
    fn resolves_omitted_default_fold_to_the_stored_matrix() {
        let line_ids = test_line_ids(&["F-F", "F-F-R2", "F-F-R2-R7.5", "F-F-R2-R7.5-A100-F"]);

        assert_eq!(
            canonical_line_transition_path("F-F-R2-R7.5-A100-F", "F-F", &line_ids, 6)
                .expect("canonical transition path"),
            (
                vec![
                    "F-F".to_owned(),
                    "F-F-R2".to_owned(),
                    "F-F-R2-R7.5".to_owned(),
                    "F-F-R2-R7.5-A100-F".to_owned(),
                ],
                1,
            )
        );
    }

    #[test]
    fn resolves_multiple_omitted_default_folds_when_the_source_requires_them() {
        let line_ids = test_line_ids(&["F-F", "F-F-R2-C-F-F"]);

        assert_eq!(
            resolve_implicit_fold_line("F-F-R2-C", &line_ids, 6),
            Some("F-F-R2-C-F-F".to_owned())
        );
    }

    fn test_line_ids(lines: &[&str]) -> HashMap<String, u32> {
        lines
            .iter()
            .enumerate()
            .map(|(index, line)| ((*line).to_owned(), index as u32))
            .collect()
    }

    #[test]
    fn computes_reuse_distance_from_distinct_intervening_matrices() {
        let key_a = WorkloadMatrixKey {
            dimension: "default:6max:100BB".to_owned(),
            concrete_line_id: 1,
        };
        let key_b = WorkloadMatrixKey {
            dimension: "default:6max:100BB".to_owned(),
            concrete_line_id: 2,
        };
        let key_c = WorkloadMatrixKey {
            dimension: "default:6max:100BB".to_owned(),
            concrete_line_id: 3,
        };
        let mut seen = HashMap::new();
        let mut history = Vec::new();
        record_workload_access(&mut seen, &mut history, key_a.clone());
        record_workload_access(&mut seen, &mut history, key_b.clone());
        record_workload_access(&mut seen, &mut history, key_c);
        record_workload_access(&mut seen, &mut history, key_b.clone());

        assert_eq!(reuse_distance(&history, &seen, &key_a), Some(2));
        assert_eq!(reuse_distance(&history, &seen, &key_b), Some(0));
    }
}
