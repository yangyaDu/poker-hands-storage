use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::benchmark::cli::{next_value, parse_usize};
use crate::benchmark::cold::types::LatencySummary;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, MemorySnapshot};
use crate::benchmark::report_support::{
    generated_at_utc, markdown_table, write_json_report, write_markdown_report,
};
use crate::benchmark::types::{BenchmarkWorkload, DrillScenarioBenchmarkItem, WorkloadOptions};
use crate::benchmark::workload::{create_benchmark_workload, read_workload_json};
use crate::errors::ToolError;

use super::cli::parse_three_way_hot_benchmark_args;
use super::query_facade::ProtoRangeStoreFacade;
use super::three_way_benchmark::{
    run_three_way_hot_benchmark, ThreeWayHotBenchmarkCommand, ThreeWayHotBenchmarkReport,
};

#[derive(Debug, Clone)]
pub struct ThreeWayStabilityBenchmarkCommand {
    pub hot: ThreeWayHotBenchmarkCommand,
    pub runs: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayStabilityBenchmarkReport {
    pub generated_at: String,
    pub runs: usize,
    pub raw_report_paths: Vec<String>,
    pub cases: Vec<ThreeWayStabilityCase>,
    pub metadata_cache: MetadataCachePhaseReport,
    pub hand_strategy_profile: ProtoHandStrategyProfileReport,
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
    pub samples: usize,
    pub facade_total_ms: LatencySummary,
    pub facade_overhead_ms: LatencySummary,
    pub dimension_check_ms: LatencySummary,
    pub parse_hand_ms: LatencySummary,
    pub matrix_read_ms: LatencySummary,
    pub matrix_cache_hits: usize,
    pub matrix_cache_misses: usize,
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

pub fn parse_three_way_stability_benchmark_args(
    args: Vec<String>,
) -> Result<ThreeWayStabilityBenchmarkCommand, ToolError> {
    let mut runs = 3usize;
    let mut hot_args = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--runs" {
            runs = parse_usize("--runs", next_value(&args, &mut index)?)?;
        } else {
            hot_args.push(args[index].clone());
        }
        index += 1;
    }
    if runs < 2 {
        return Err(ToolError::invalid_argument("--runs must be at least 2"));
    }
    Ok(ThreeWayStabilityBenchmarkCommand {
        hot: parse_three_way_hot_benchmark_args(hot_args)?,
        runs,
    })
}

pub fn run_three_way_stability_benchmark(
    command: &ThreeWayStabilityBenchmarkCommand,
) -> Result<ThreeWayStabilityBenchmarkReport, ToolError> {
    let mut reports = Vec::with_capacity(command.runs);
    let mut raw_report_paths = Vec::with_capacity(command.runs);
    for run_index in 1..=command.runs {
        let mut run_command = command.hot.clone();
        run_command.out_path = per_run_path(&command.hot.out_path, run_index);
        run_command.md_path = per_run_path(&command.hot.md_path, run_index);
        let report = run_three_way_hot_benchmark(&run_command)?;
        raw_report_paths.push(run_command.out_path.display().to_string());
        reports.push(report);
    }
    let report = ThreeWayStabilityBenchmarkReport {
        generated_at: generated_at_utc(),
        runs: command.runs,
        raw_report_paths,
        cases: summarize_cases(&reports),
        metadata_cache: measure_metadata_cache_phases(&command.hot)?,
        hand_strategy_profile: profile_hand_strategy_tail(&command.hot)?,
        notes: vec![
            "Every run uses the same workload seed or supplied workload file; raw reports are retained beside this summary.".to_owned(),
            "Metadata phases use one Proto facade with max_open_handles=1: first query, cache hit, then optional query after LRU eviction.".to_owned(),
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

fn profile_hand_strategy_tail(
    command: &ThreeWayHotBenchmarkCommand,
) -> Result<ProtoHandStrategyProfileReport, ToolError> {
    let workload = load_workload(command)?;
    let facade = ProtoRangeStoreFacade::open(
        &command.proto_root,
        command.max_open_handles,
        command.verify_checksums,
    )?;
    for item in &workload.hand_queries {
        facade.prewarm(&item.dimension())?;
    }
    for item in workload.hand_queries.iter().take(command.warmup_iterations) {
        facade.profile_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)?;
    }

    let mut facade_total = Vec::with_capacity(workload.hand_queries.len());
    let mut facade_overhead = Vec::with_capacity(workload.hand_queries.len());
    let mut dimension_check = Vec::with_capacity(workload.hand_queries.len());
    let mut parse_hand = Vec::with_capacity(workload.hand_queries.len());
    let mut matrix_read = Vec::with_capacity(workload.hand_queries.len());
    let mut matrix_cache_hits = 0usize;
    let mut matrix_index_payload = Vec::new();
    let mut matrix_protobuf_decode = Vec::new();
    let mut matrix_compact_index = Vec::new();
    let mut matrix_cache_insert = Vec::new();
    let mut action_materialization = Vec::with_capacity(workload.hand_queries.len());
    let mut service_total = Vec::with_capacity(workload.hand_queries.len());
    let mut slowest = Vec::with_capacity(workload.hand_queries.len());
    for item in &workload.hand_queries {
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
            matrix_index_payload.push(phases.matrix_index_payload_ms);
            matrix_protobuf_decode.push(phases.matrix_protobuf_decode_ms);
            matrix_compact_index.push(phases.matrix_compact_index_ms);
            matrix_cache_insert.push(phases.matrix_cache_insert_ms);
        }
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
    }
    slowest.sort_by(|left, right| right.facade_total_ms.total_cmp(&left.facade_total_ms));
    slowest.truncate(10);
    Ok(ProtoHandStrategyProfileReport {
        samples: facade_total.len(),
        facade_total_ms: LatencySummary::from_values(&facade_total),
        facade_overhead_ms: LatencySummary::from_values(&facade_overhead),
        dimension_check_ms: LatencySummary::from_values(&dimension_check),
        parse_hand_ms: LatencySummary::from_values(&parse_hand),
        matrix_read_ms: LatencySummary::from_values(&matrix_read),
        matrix_cache_hits,
        matrix_cache_misses: facade_total.len() - matrix_cache_hits,
        matrix_index_payload_ms: LatencySummary::from_values(&matrix_index_payload),
        matrix_protobuf_decode_ms: LatencySummary::from_values(&matrix_protobuf_decode),
        matrix_compact_index_ms: LatencySummary::from_values(&matrix_compact_index),
        matrix_cache_insert_ms: LatencySummary::from_values(&matrix_cache_insert),
        action_materialization_ms: LatencySummary::from_values(&action_materialization),
        service_total_ms: LatencySummary::from_values(&service_total),
        slowest,
    })
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
    let mut markdown = String::from("# Core vs Proto vs SQLite Stability Benchmark Report\n\n");
    markdown.push_str(&format!("- Repetitions: `{}`\n\n", report.runs));
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
            format!("{:.4}", report.metadata_cache.first_query_ms),
            format!("{:.4}", report.metadata_cache.cache_hit_ms),
            report
                .metadata_cache
                .post_eviction_query_ms
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "n/a".to_owned()),
            format_optional_bytes(report.metadata_cache.first_query_rss_delta_bytes),
            format_optional_bytes(report.metadata_cache.cache_hit_rss_delta_bytes),
            format_optional_bytes(report.metadata_cache.post_eviction_rss_delta_bytes),
        ]],
    ));
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
