use std::path::PathBuf;

use poker_hands_storage_tools::benchmark::cli::{next_value, parse_u32};
use poker_hands_storage_tools::benchmark::cold::{
    parse_benchmark_cold_args, parse_benchmark_cold_compare_args, parse_benchmark_sqlite_cold_args,
    run_cold_start_compare, run_sqlite_cold_benchmark,
};
use poker_hands_storage_tools::benchmark::hot::{
    parse_benchmark_args, parse_benchmark_compare_args, parse_benchmark_sqlite_args,
    run_benchmark_compare, run_sqlite_benchmark,
};
use poker_hands_storage_tools::benchmark::native::parse_benchmark_native_args;
use poker_hands_storage_tools::benchmark::native::run_core_worker_from_input_path;
use poker_hands_storage_tools::benchmark::run_cold_benchmark;
use poker_hands_storage_tools::benchmark::run_drill_metadata_benchmark;
use poker_hands_storage_tools::benchmark::run_hot_benchmark;
use poker_hands_storage_tools::benchmark::run_native_benchmark;
use poker_hands_storage_tools::compact_line_matrix_archive::cli::{
    parse_benchmark_compact_vs_core_args, parse_compact_vs_core_cold_worker_args,
    parse_export_all_compact_line_matrix_archives_args,
    parse_export_compact_line_matrix_archive_args, parse_verify_compact_line_matrix_archive_args,
};
use poker_hands_storage_tools::compact_line_matrix_archive::{
    export_all_compact_line_matrix_archives, export_compact_line_matrix_archive,
    run_compact_vs_core_benchmark, run_compact_vs_core_cold_worker, CompactLineMatrixArchive,
};
use poker_hands_storage_tools::errors::ToolError;
use poker_hands_storage_tools::line_matrix_archive::cli::parse_export_line_matrix_archive_args;
use poker_hands_storage_tools::line_matrix_archive::export_line_matrix_archive;
use poker_hands_storage_tools::line_matrix_export::cli::parse_export_line_matrix_args;
use poker_hands_storage_tools::line_matrix_export::export_line_matrix;
use poker_hands_storage_tools::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_tools::verification::cli::parse_verify_args;
use poker_hands_storage_tools::verification::cross::{run_cross_verify, CrossVerifyOptions};
use poker_hands_storage_tools::verification::report::{RangeStrataVerifyReport, VerifyMode};
use poker_hands_storage_tools::verification::standalone::{
    run_standalone_verify, StandaloneVerifyOptions,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ToolError> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("build") => run_build(args.collect()),
        Some("export-compact-line-matrix-archive") => {
            run_export_compact_line_matrix_archive(args.collect())
        }
        Some("export-all-compact-line-matrix-archives") => {
            run_export_all_compact_line_matrix_archives(args.collect())
        }
        Some("verify-compact-line-matrix-archive") => {
            run_verify_compact_line_matrix_archive(args.collect())
        }
        Some("benchmark-compact-vs-core") => run_benchmark_compact_vs_core(args.collect()),
        Some("compact-vs-core-cold-worker") => run_compact_vs_core_cold_worker_cmd(args.collect()),
        Some("export-line-matrix-archive") => run_export_line_matrix_archive(args.collect()),
        Some("export-line-matrix") => run_export_line_matrix(args.collect()),
        Some("verify") => run_verify(args.collect()),
        Some("benchmark") => run_benchmark(args.collect()),
        Some("benchmark-drill-metadata") => run_benchmark_drill_metadata(args.collect()),
        Some("benchmark-native") => run_benchmark_native(args.collect()),
        Some("benchmark-native-core-worker") => run_benchmark_native_core_worker(args.collect()),
        Some("benchmark-native-http-worker") => run_benchmark_native_http_worker(args.collect()),
        Some("benchmark-sqlite") => run_benchmark_sqlite(args.collect()),
        Some("benchmark-compare") => run_benchmark_compare_cmd(args.collect()),
        Some("benchmark-cold") => run_benchmark_cold(args.collect()),
        Some("benchmark-sqlite-cold") => run_benchmark_sqlite_cold(args.collect()),
        Some("benchmark-cold-compare") => run_benchmark_cold_compare_cmd(args.collect()),
        Some("cold-worker") => run_cold_worker_cmd(args.collect()),
        Some("sqlite-cold-worker") => run_sqlite_cold_worker_cmd(args.collect()),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(cmd) => Err(ToolError::invalid_argument(format!(
            "Unknown command: {cmd}"
        ))),
    }
}

fn run_benchmark_compact_vs_core(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_compact_vs_core_args(args)?;
    let report = run_compact_vs_core_benchmark(&command)?;
    println!("CompactLineMatrix V2 vs core benchmark complete.");
    println!("  Dimension: {}", report.dimension);
    println!(
        "  Hot compact/core P95 ratio: {:.2}x",
        report.hot.compact_to_core_p95_ratio
    );
    println!(
        "  Cold compact/core open+first P95 ratio: {:.2}x",
        report.cold.compact_to_core_open_and_first_query_p95_ratio
    );
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "COMPACT_VS_CORE_BENCHMARK_FAILED",
            "compact/core benchmark had errors",
        ));
    }
    Ok(())
}

fn run_compact_vs_core_cold_worker_cmd(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_compact_vs_core_cold_worker_args(args)?;
    let output = run_compact_vs_core_cold_worker(&command);
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| ToolError::invalid_format(error.to_string()))?
    );
    if !output.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn run_export_line_matrix_archive(args: Vec<String>) -> Result<(), ToolError> {
    let options = parse_export_line_matrix_archive_args(args)?;
    let summary = export_line_matrix_archive(&options)?;
    println!("LineMatrix archive export complete.");
    println!("  Dimension: default:6:100");
    println!("  Matrix count: {}", summary.matrix_count);
    println!("  Protobuf bytes: {}", summary.protobuf_bytes);
    println!("  Manifest: {}", summary.manifest_path.display());
    println!("  Data: {}", summary.data_path.display());
    println!("  Index: {}", summary.index_path.display());
    println!("  Metadata: {}", summary.metadata_path.display());
    Ok(())
}

fn run_export_compact_line_matrix_archive(args: Vec<String>) -> Result<(), ToolError> {
    let options = parse_export_compact_line_matrix_archive_args(args)?;
    let summary = export_compact_line_matrix_archive(&options)?;
    println!("Compact LineMatrix archive export complete.");
    println!(
        "  Dimension: {}:{}:{}",
        summary.strategy, summary.player_count, summary.depth_bb
    );
    println!("  Matrix count: {}", summary.matrix_count);
    println!("  Action values: {}", summary.action_value_count);
    println!("  Protobuf bytes: {}", summary.protobuf_bytes);
    println!("  Manifest: {}", summary.manifest_path.display());
    println!("  Data: {}", summary.data_path.display());
    println!("  Index: {}", summary.index_path.display());
    println!("  Metadata: {}", summary.metadata_path.display());
    Ok(())
}

fn run_export_all_compact_line_matrix_archives(args: Vec<String>) -> Result<(), ToolError> {
    let options = parse_export_all_compact_line_matrix_archives_args(args)?;
    let report = export_all_compact_line_matrix_archives(&options)?;
    println!("All Compact LineMatrix archives exported and verified.");
    for dimension in &report.dimensions {
        println!(
            "  {}:{}:{} matrices={} values={} lmbin={} lmidx={} bin+idx={}",
            dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            dimension.matrix_count,
            dimension.action_value_count,
            dimension.data_bytes,
            dimension.index_bytes,
            dimension.bin_idx_bytes
        );
    }
    println!("  SQLite bytes: {}", report.sqlite_bytes);
    println!("  Total lmbin bytes: {}", report.total_data_bytes);
    println!("  Total lmidx bytes: {}", report.total_index_bytes);
    println!("  Total bin+idx bytes: {}", report.total_bin_idx_bytes);
    println!(
        "  bin+idx / SQLite: {:.6}%",
        report.bin_idx_to_sqlite_percent
    );
    println!(
        "  SQLite share of SQLite+bin+idx: {:.6}%",
        report.sqlite_share_percent
    );
    println!(
        "  bin+idx share of SQLite+bin+idx: {:.6}%",
        report.bin_idx_share_percent
    );
    println!("  Report: {}", report.report_path.display());
    Ok(())
}

fn run_verify_compact_line_matrix_archive(args: Vec<String>) -> Result<(), ToolError> {
    let dir = parse_verify_compact_line_matrix_archive_args(args)?;
    let archive = CompactLineMatrixArchive::open(&dir)?;
    let summary = archive.verify_all()?;
    println!("Compact LineMatrix archive verification complete.");
    println!("  Matrix count: {}", summary.matrix_count);
    println!("  Action count: {}", summary.action_count);
    println!("  Action values: {}", summary.action_value_count);
    Ok(())
}

fn run_export_line_matrix(args: Vec<String>) -> Result<(), ToolError> {
    let options = parse_export_line_matrix_args(args)?;
    let summary = export_line_matrix(&options)?;
    println!("LineMatrix protobuf export complete.");
    println!("  Concrete line id: {}", summary.concrete_line_id);
    println!("  Abstract line: {}", summary.abstract_line);
    println!("  Concrete line: {}", summary.concrete_line);
    println!("  Actions: {}", summary.action_count);
    println!("  Source rows: {}", summary.source_row_count);
    println!("  NULL EV cells: {}", summary.null_ev_count);
    println!("  Hands with actions: {}", summary.hands_with_actions);
    println!("  Hands without actions: {}", summary.hands_without_actions);
    println!(
        "  Frequency sum mismatches: {}",
        summary.frequency_sum_mismatch_hand_count
    );
    println!(
        "  Max frequency error x10000: {}",
        summary.max_frequency_error_x10000
    );
    println!("  Protobuf bytes: {}", summary.protobuf_bytes);
    println!("  Protobuf: {}", summary.protobuf_path.display());
    println!("  Debug JSON: {}", summary.debug_json_path.display());
    println!("  Verify JSON: {}", summary.verify_json_path.display());
    Ok(())
}

fn run_build(args: Vec<String>) -> Result<(), ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut dimensions = Vec::new();
    let mut max_concrete_lines = None;
    let mut overwrite = false;
    let mut resume = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => dimensions.push(DimensionSpec::parse(next_value(&args, &mut index)?)?),
            "--max-concrete-lines" => {
                max_concrete_lines = Some(next_value(&args, &mut index)?.parse().map_err(|_| {
                    ToolError::invalid_argument("--max-concrete-lines must be an integer")
                })?)
            }
            "--overwrite" => overwrite = true,
            "--resume" => resume = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown build option: {option}"
                )))
            }
        }
        index += 1;
    }
    let summary = build_store(&BuildOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        dimensions,
        max_concrete_lines_per_dimension: max_concrete_lines,
        overwrite,
        resume,
    })?;
    println!("manifest={}", summary.manifest_path.display());
    for dimension in summary.dimensions {
        println!(
            "dimension={}:{}:{} packs={} bin_bytes={} idx_bytes={}",
            dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
            dimension.pack_count,
            dimension.bin_file_size_bytes.unwrap_or_default(),
            dimension.idx_file_size_bytes.unwrap_or_default()
        );
    }
    Ok(())
}

fn run_verify(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_verify_args(args)?;
    let report = match command.mode {
        VerifyMode::Standalone => run_standalone_verify(&StandaloneVerifyOptions {
            dir: command.dir,
            verify_checksums: command.verify_checksums,
            out_path: Some(command.out_path),
            md_path: Some(command.md_path),
        })?,
        VerifyMode::Cross => {
            let source = command.source.ok_or_else(|| {
                ToolError::invalid_argument("--source is required for cross mode")
            })?;
            run_cross_verify(&CrossVerifyOptions {
                dir: command.dir,
                source_db: source,
                sample_size: command.sample_size,
                max_failures: command.max_failures,
                verify_checksums: command.verify_checksums,
                out_path: Some(command.out_path),
                md_path: Some(command.md_path),
            })?
        }
    };
    print_verify_summary(&report);
    Ok(())
}

fn run_benchmark(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_args(args)?;
    let report = run_hot_benchmark(&command)?;
    println!("Range Strata Binary benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new("BENCHMARK_FAILED", "benchmark failed"));
    }
    Ok(())
}

fn run_benchmark_drill_metadata(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_args(args)?;
    let report = run_drill_metadata_benchmark(&command)?;
    println!("Drill metadata benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "BENCHMARK_DRILL_METADATA_FAILED",
            "drill metadata benchmark failed",
        ));
    }
    Ok(())
}

fn run_benchmark_native(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_native_args(args)?;
    let report = run_native_benchmark(&command)?;
    println!("Bun native benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "BENCHMARK_NATIVE_FAILED",
            "Bun native benchmark failed",
        ));
    }
    Ok(())
}

fn run_benchmark_native_core_worker(args: Vec<String>) -> Result<(), ToolError> {
    let input_path = args
        .first()
        .ok_or_else(|| ToolError::invalid_argument("worker input path is required"))?;
    let output = run_core_worker_from_input_path(PathBuf::from(input_path).as_path())?;
    println!("{output}");
    Ok(())
}

fn run_benchmark_native_http_worker(args: Vec<String>) -> Result<(), ToolError> {
    let input_path = args
        .first()
        .ok_or_else(|| ToolError::invalid_argument("worker input path is required"))?;
    let output = poker_hands_storage_tools::benchmark::native::run_http_worker_from_input_path(
        PathBuf::from(input_path).as_path(),
    )?;
    println!("{output}");
    Ok(())
}

fn run_benchmark_cold(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_cold_args(args)?;
    let report = run_cold_benchmark(&command)?;
    println!("Range Strata Binary cold-start benchmark complete.");
    println!("  Dimensions: {}", report.aggregate.dimensions);
    println!("  Total runs: {}", report.aggregate.runs);
    println!("  Errors: {}", report.aggregate.error_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "BENCHMARK_COLD_FAILED",
            "cold-start benchmark had errors",
        ));
    }
    Ok(())
}

fn run_benchmark_sqlite_cold(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_sqlite_cold_args(args)?;
    let report = run_sqlite_cold_benchmark(&command)?;
    println!("SQLite cold-start benchmark complete.");
    println!("  Dimensions: {}", report.aggregate.dimensions);
    println!("  Total runs: {}", report.aggregate.runs);
    println!("  Errors: {}", report.aggregate.error_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "BENCHMARK_SQLITE_COLD_FAILED",
            "SQLite cold-start benchmark had errors",
        ));
    }
    Ok(())
}

fn run_benchmark_sqlite(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_sqlite_args(args)?;
    let report = run_sqlite_benchmark(&command)?;
    println!("SQLite benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(ToolError::new(
            "BENCHMARK_SQLITE_FAILED",
            "SQLite benchmark failed",
        ));
    }
    Ok(())
}

fn run_benchmark_compare_cmd(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_compare_args(args)?;
    let report = run_benchmark_compare(&command)?;
    println!("Benchmark comparison complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Compatible workload: {}", report.compatible_workload);
    println!(
        "  Compatibility notes: {}",
        report.compatibility_notes.len()
    );
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    Ok(())
}

fn run_benchmark_cold_compare_cmd(args: Vec<String>) -> Result<(), ToolError> {
    let command = parse_benchmark_cold_compare_args(args)?;
    let report = run_cold_start_compare(&command)?;
    println!("Cold-start benchmark comparison complete.");
    println!("  Dimensions: {}", report.dimensions.len());
    println!("  Compatible: {}", report.compatible);
    println!(
        "  Compatibility notes: {}",
        report.compatibility_notes.len()
    );
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    Ok(())
}

fn run_cold_worker_cmd(args: Vec<String>) -> Result<(), ToolError> {
    let mut dir = None;
    let mut meta = None;
    let mut strategy = "default".to_owned();
    let mut player_count = None;
    let mut depth_bb = None;
    let mut concrete_line_id = None;
    let mut hand = None;
    let mut verify_checksum = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--meta" => meta = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--strategy" => strategy = next_value(&args, &mut index)?.to_owned(),
            "--player-count" => {
                player_count = Some(parse_u32("--player-count", next_value(&args, &mut index)?)?)
            }
            "--depth-bb" => {
                depth_bb = Some(parse_u32("--depth-bb", next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand" => hand = Some(next_value(&args, &mut index)?.to_owned()),
            "--verify-checksum" => verify_checksum = true,
            _ => {} // Ignore unknown args silently in worker
        }
        index += 1;
    }
    let dir = dir.ok_or_else(|| ToolError::invalid_argument("--dir is required"))?;
    let meta = meta.unwrap_or_else(|| dir.join("meta.db"));
    let player_count =
        player_count.ok_or_else(|| ToolError::invalid_argument("--player-count is required"))?;
    let depth_bb = depth_bb.ok_or_else(|| ToolError::invalid_argument("--depth-bb is required"))?;
    let concrete_line_id = concrete_line_id
        .ok_or_else(|| ToolError::invalid_argument("--concrete-line-id is required"))?;
    let hand = hand.ok_or_else(|| ToolError::invalid_argument("--hand is required"))?;

    let output = poker_hands_storage_tools::benchmark::cold::worker::run_cold_worker(
        &poker_hands_storage_tools::benchmark::cold::worker::ColdWorkerParams {
            dir: &dir,
            meta: &meta,
            strategy: &strategy,
            player_count,
            depth_bb,
            concrete_line_id,
            hand: &hand,
            verify_checksums: verify_checksum,
        },
    );
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| ToolError::invalid_format(e.to_string()))?
    );
    if !output.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn run_sqlite_cold_worker_cmd(args: Vec<String>) -> Result<(), ToolError> {
    let mut source = None;
    let mut strategy = "default".to_owned();
    let mut player_count = None;
    let mut depth_bb = None;
    let mut concrete_line_id = None;
    let mut hand = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--strategy" => strategy = next_value(&args, &mut index)?.to_owned(),
            "--player-count" => {
                player_count = Some(parse_u32("--player-count", next_value(&args, &mut index)?)?)
            }
            "--depth-bb" => {
                depth_bb = Some(parse_u32("--depth-bb", next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand" => hand = Some(next_value(&args, &mut index)?.to_owned()),
            _ => {}
        }
        index += 1;
    }
    let source = source.ok_or_else(|| ToolError::invalid_argument("--source is required"))?;
    let player_count =
        player_count.ok_or_else(|| ToolError::invalid_argument("--player-count is required"))?;
    let depth_bb = depth_bb.ok_or_else(|| ToolError::invalid_argument("--depth-bb is required"))?;
    let concrete_line_id = concrete_line_id
        .ok_or_else(|| ToolError::invalid_argument("--concrete-line-id is required"))?;
    let hand = hand.ok_or_else(|| ToolError::invalid_argument("--hand is required"))?;

    let output = poker_hands_storage_tools::benchmark::cold::sqlite_worker::run_sqlite_cold_worker(
        &poker_hands_storage_tools::benchmark::cold::sqlite_worker::SqliteColdWorkerParams {
            source: &source,
            strategy: &strategy,
            player_count,
            depth_bb,
            concrete_line_id,
            hand: &hand,
        },
    );
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| ToolError::invalid_format(e.to_string()))?
    );
    if !output.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn print_verify_summary(report: &RangeStrataVerifyReport) {
    println!("mode={:?}", report.mode);
    println!("pass={}", report.failures.is_empty());
    println!("failures={}", report.failures.len());
}

fn print_help() {
    println!(
        "poker-hands-storage-tools

Commands:
  build --source-db <range.db> --out-dir <dir>
        [--dimension strategy:player_count:depth_bb]
        [--max-concrete-lines <count>] [--overwrite] [--resume]

  export-line-matrix --source-db <range.db> --out-dir <dir>
        --dimension <strategy:player_count:depth_bb>
        (--concrete-line-id <id> | --concrete-line <line>)
        [--abstract-line <line>] --gto-data-version <version> [--overwrite]

  export-line-matrix-archive --source-db <range.db> --out-dir <dir>
        --gto-data-version <version> [--overwrite]
        Exports every LineMatrix for default:6:100.

  export-compact-line-matrix-archive --source-db <range.db> --out-dir <dir>
        [--dimension strategy:player_count:depth_bb] [--overwrite]
        Exports one V2 CompactLineMatrix dimension (default:6:100).

  export-all-compact-line-matrix-archives --source-db <range.db> --out-dir <dir>
        [--overwrite]
        Discovers, exports, verifies, and reports every V2 dimension.

  verify-compact-line-matrix-archive --dir <dir>
        Verifies every V2 record checksum, payload, and compact index.

  benchmark-compact-vs-core --compact-dir <compact-archive-dir> --core-dir <range-strata-dir>
        --dimension <strategy:player_count:depth_bb>
        [--hot-iterations <count>] [--warmup-iterations <count>]
        [--cold-runs <count>] [--cold-mode <process-cold|os-best-effort|linux-drop-cache>]
        [--cache-filler-mb <mb>] [--seed <number>] [--max-open-handles <count>]
        [--concrete-line-id <id> --hand-id <0..168>] [--verify-checksum]
        [--out <report.json>] [--md <report.md>]

  verify --dir <dir> [--mode standalone|cross] [--source <range.db>]
         [--verify-checksum] [--sample-size <n>] [--max-failures <n>]
         [--out <report.json>] [--md <report.md>]

  benchmark --dir <dir> --source <range.db>
        [--meta <meta.db>] [--workload <workload.json>]
        [--seed <number>] [--iterations <count>]
        [--hand-iterations <count>] [--batch-iterations <count>]
        [--batch-size <count>] [--batch-sizes <csv>]
        [--dimension <strategy:players:bb>]
        [--workload-mode <random|abstract-local>]
        [--write-workload <workload.json>]
        [--warmup-iterations <count>] [--verify-checksum]
        [--verify-results] [--out <report.json>] [--md <report.md>]

  benchmark-drill-metadata --dir <dir> --source <range.db>
        [--meta <meta.db>] [--workload <workload.json>]
        [--seed <number>] [--iterations <count>]
        [--dimension <strategy:players:bb>]
        [--write-workload <workload.json>]
        [--warmup-iterations <count>]
        [--out <report.json>] [--md <report.md>]

  benchmark-native --dir <dir> --source <range.db>
        [--meta <meta.db>] [--native-entry <range-store-native/index.js>]
        [--http-service-bin <poker-hands-storage-service>] [--bun <bun>]
        [--max-open-handles <count>]
        [--workload <workload.json>] [--seed <number>]
        [--iterations <count>] [--hand-iterations <count>]
        [--batch-iterations <count>] [--batch-size <count>]
        [--batch-sizes <csv>] [--dimension <strategy:players:bb>]
        [--workload-mode <random|abstract-local>]
        [--write-workload <workload.json>]
        [--warmup-iterations <count>] [--verify-checksum]
        [--out <report.json>] [--md <report.md>]

  benchmark-sqlite --source <range.db>
        [--workload <workload.json>] [--seed <number>]
        [--iterations <count>] [--hand-iterations <count>]
        [--batch-iterations <count>] [--batch-size <count>]
        [--batch-sizes <csv>] [--dimension <strategy:players:bb>]
        [--workload-mode <random|abstract-local>]
        [--warmup-iterations <count>] [--out <report.json>] [--md <report.md>]

  benchmark-compare --binary <binary-report.json> --sqlite <sqlite-report.json>
        [--allow-mismatch] [--out <report.json>] [--md <report.md>]

  benchmark-cold --dir <dir> --source <range.db>
        [--meta <meta.db>] [--mode <process-cold|os-best-effort|linux-drop-cache>]
        [--runs <count>] [--dimension <strategy:players:bb>]
        [--query-policy <first|fixed>] [--concrete-line-id <id>] [--hand <hand>]
        [--cache-filler-mb <mb>] [--max-errors-per-dimension <count>]
        [--fail-fast] [--verify-checksum]
        [--out <report.json>] [--md <report.md>]

  benchmark-sqlite-cold --dir <dir> --source <range.db>
        [--mode <process-cold|os-best-effort|linux-drop-cache>]
        [--runs <count>] [--dimension <strategy:players:bb>]
        [--query-policy <first|fixed>] [--concrete-line-id <id>] [--hand <hand>]
        [--cache-filler-mb <mb>] [--max-errors-per-dimension <count>]
        [--fail-fast] [--out <report.json>] [--md <report.md>]

  benchmark-cold-compare --binary <binary-cold-report.json> --sqlite <sqlite-cold-report.json>
        [--allow-mismatch] [--out <report.json>] [--md <report.md>]

  help  Print this help message"
    );
}
