use std::env;
use std::path::PathBuf;

use poker_hands_storage_service::benchmark::cold::{
    parse_benchmark_cold_args, parse_benchmark_cold_compare_args, parse_benchmark_sqlite_cold_args,
    run_cold_start_compare, run_sqlite_cold_benchmark,
};
use poker_hands_storage_service::benchmark::compare::{
    parse_benchmark_compare_args, run_benchmark_compare,
};
use poker_hands_storage_service::benchmark::hot::parse_benchmark_args;
use poker_hands_storage_service::benchmark::run_cold_benchmark;
use poker_hands_storage_service::benchmark::run_hot_benchmark;
use poker_hands_storage_service::benchmark::sqlite::{
    parse_benchmark_sqlite_args, run_sqlite_benchmark,
};
use poker_hands_storage_service::config::ServiceConfig;
use poker_hands_storage_service::domain::dimension::DimensionRef;
use poker_hands_storage_service::errors::AppError;
use poker_hands_storage_service::http;
use poker_hands_storage_service::http::healthcheck::{
    run_http_healthcheck, HttpHealthcheckOptions,
};
use poker_hands_storage_service::query::QueryService;
use poker_hands_storage_service::range_store_builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_service::verification::cli::parse_verify_args;
use poker_hands_storage_service::verification::cross::{run_cross_verify, CrossVerifyOptions};
use poker_hands_storage_service::verification::report::{RangeStrataVerifyReport, VerifyMode};
use poker_hands_storage_service::verification::standalone::{
    run_standalone_verify, StandaloneVerifyOptions,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), AppError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("build") => run_build(args.collect()),
        Some("query") => run_query(args.collect()),
        Some("verify") => run_verify(args.collect()),
        Some("benchmark") => run_benchmark(args.collect()),
        Some("benchmark-sqlite") => run_benchmark_sqlite(args.collect()),
        Some("benchmark-compare") => run_benchmark_compare_cmd(args.collect()),
        Some("benchmark-cold") => run_benchmark_cold(args.collect()),
        Some("benchmark-sqlite-cold") => run_benchmark_sqlite_cold(args.collect()),
        Some("benchmark-cold-compare") => run_benchmark_cold_compare_cmd(args.collect()),
        Some("cold-worker") => run_cold_worker_cmd(args.collect()),
        Some("sqlite-cold-worker") => run_sqlite_cold_worker_cmd(args.collect()),
        Some("healthcheck") => run_healthcheck(args.collect()),
        Some("serve") => {
            let remaining: Vec<_> = args.collect();
            if !remaining.is_empty() {
                return Err(AppError::invalid_argument(format!(
                    "Unknown serve option: {}",
                    remaining.join(" ")
                )));
            }
            http::serve(ServiceConfig::from_env()?).await
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(AppError::invalid_argument(format!(
            "Unknown command: {command}"
        ))),
    }
}

fn run_benchmark(args: Vec<String>) -> Result<(), AppError> {
    let command = parse_benchmark_args(args)?;
    let report = run_hot_benchmark(&command)?;
    println!("Range Strata Binary benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result action count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(AppError::new("BENCHMARK_FAILED", "benchmark failed"));
    }
    Ok(())
}

fn run_benchmark_cold(args: Vec<String>) -> Result<(), AppError> {
    let command = parse_benchmark_cold_args(args)?;
    let report = run_cold_benchmark(&command)?;
    println!("Range Strata Binary cold-start benchmark complete.");
    println!("  Dimensions: {}", report.aggregate.dimensions);
    println!("  Total runs: {}", report.aggregate.runs);
    println!("  Errors: {}", report.aggregate.error_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(AppError::new(
            "BENCHMARK_COLD_FAILED",
            "cold-start benchmark had errors",
        ));
    }
    Ok(())
}

fn run_benchmark_sqlite_cold(args: Vec<String>) -> Result<(), AppError> {
    let command = parse_benchmark_sqlite_cold_args(args)?;
    let report = run_sqlite_cold_benchmark(&command)?;
    println!("SQLite cold-start benchmark complete.");
    println!("  Dimensions: {}", report.aggregate.dimensions);
    println!("  Total runs: {}", report.aggregate.runs);
    println!("  Errors: {}", report.aggregate.error_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(AppError::new(
            "BENCHMARK_SQLITE_COLD_FAILED",
            "SQLite cold-start benchmark had errors",
        ));
    }
    Ok(())
}

fn run_benchmark_sqlite(args: Vec<String>) -> Result<(), AppError> {
    let command = parse_benchmark_sqlite_args(args)?;
    let report = run_sqlite_benchmark(&command)?;
    println!("SQLite benchmark complete.");
    println!("  Cases: {}", report.cases.len());
    println!("  Total iterations: {}", report.totals.iterations);
    println!("  Aggregate QPS: {:.2}", report.totals.avg_qps);
    println!("  Error count: {}", report.totals.error_count);
    println!("  Result action count: {}", report.totals.result_count);
    println!("  JSON report: {}", command.out_path.display());
    println!("  Markdown report: {}", command.md_path.display());
    if report.has_errors() {
        return Err(AppError::new(
            "BENCHMARK_SQLITE_FAILED",
            "SQLite benchmark failed",
        ));
    }
    Ok(())
}

fn run_benchmark_compare_cmd(args: Vec<String>) -> Result<(), AppError> {
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

fn run_benchmark_cold_compare_cmd(args: Vec<String>) -> Result<(), AppError> {
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

fn run_cold_worker_cmd(args: Vec<String>) -> Result<(), AppError> {
    // Parse minimal args for the cold worker subprocess.
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
            "--dir" => dir = Some(std::path::PathBuf::from(next_value(&args, &mut index)?)),
            "--meta" => meta = Some(std::path::PathBuf::from(next_value(&args, &mut index)?)),
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
    let dir = dir.ok_or_else(|| AppError::invalid_argument("--dir is required"))?;
    let meta = meta.unwrap_or_else(|| dir.join("meta.db"));
    let player_count =
        player_count.ok_or_else(|| AppError::invalid_argument("--player-count is required"))?;
    let depth_bb = depth_bb.ok_or_else(|| AppError::invalid_argument("--depth-bb is required"))?;
    let concrete_line_id = concrete_line_id
        .ok_or_else(|| AppError::invalid_argument("--concrete-line-id is required"))?;
    let hand = hand.ok_or_else(|| AppError::invalid_argument("--hand is required"))?;

    // Suppress tracing output in worker — only stdout JSON matters.
    let output = poker_hands_storage_service::benchmark::cold::worker::run_cold_worker(
        &poker_hands_storage_service::benchmark::cold::worker::ColdWorkerParams {
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
    // Output JSON to stdout — this is what the parent process reads.
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|e| AppError::invalid_format(e.to_string()))?
    );
    if !output.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn run_sqlite_cold_worker_cmd(args: Vec<String>) -> Result<(), AppError> {
    let mut source = None;
    let mut strategy = "default".to_owned();
    let mut player_count = None;
    let mut depth_bb = None;
    let mut concrete_line_id = None;
    let mut hand = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(std::path::PathBuf::from(next_value(&args, &mut index)?)),
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
    let source = source.ok_or_else(|| AppError::invalid_argument("--source is required"))?;
    let player_count =
        player_count.ok_or_else(|| AppError::invalid_argument("--player-count is required"))?;
    let depth_bb = depth_bb.ok_or_else(|| AppError::invalid_argument("--depth-bb is required"))?;
    let concrete_line_id = concrete_line_id
        .ok_or_else(|| AppError::invalid_argument("--concrete-line-id is required"))?;
    let hand = hand.ok_or_else(|| AppError::invalid_argument("--hand is required"))?;

    let output =
        poker_hands_storage_service::benchmark::cold::sqlite_worker::run_sqlite_cold_worker(
            &poker_hands_storage_service::benchmark::cold::sqlite_worker::SqliteColdWorkerParams {
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
        serde_json::to_string(&output).map_err(|e| AppError::invalid_format(e.to_string()))?
    );
    if !output.ok {
        std::process::exit(1);
    }
    Ok(())
}

fn run_healthcheck(args: Vec<String>) -> Result<(), AppError> {
    let mut options = HttpHealthcheckOptions::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--url" => options.url = next_value(&args, &mut index)?.to_owned(),
            "--timeout-ms" => {
                let timeout_ms = next_value(&args, &mut index)?
                    .parse::<u64>()
                    .map_err(|_| AppError::invalid_argument("--timeout-ms must be an integer"))?;
                if timeout_ms == 0 {
                    return Err(AppError::invalid_argument(
                        "--timeout-ms must be greater than 0",
                    ));
                }
                options.timeout = std::time::Duration::from_millis(timeout_ms);
            }
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown healthcheck option: {option}"
                )))
            }
        }
        index += 1;
    }
    run_http_healthcheck(&options)
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn run_build(args: Vec<String>) -> Result<(), AppError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut dimensions = Vec::new();
    let mut max_concrete_lines = None;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => dimensions.push(DimensionSpec::parse(next_value(&args, &mut index)?)?),
            "--max-concrete-lines" => {
                max_concrete_lines = Some(next_value(&args, &mut index)?.parse().map_err(|_| {
                    AppError::invalid_argument("--max-concrete-lines must be an integer")
                })?)
            }
            "--overwrite" => overwrite = true,
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown build option: {option}"
                )))
            }
        }
        index += 1;
    }
    let summary = build_store(&BuildOptions {
        source_db: source_db
            .ok_or_else(|| AppError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| AppError::invalid_argument("--out-dir is required"))?,
        dimensions,
        max_concrete_lines_per_dimension: max_concrete_lines,
        overwrite,
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

fn run_verify(args: Vec<String>) -> Result<(), AppError> {
    let command = parse_verify_args(args)?;
    let report = match command.mode {
        VerifyMode::Standalone => run_standalone_verify(&StandaloneVerifyOptions {
            dir: command.dir,
            verify_checksums: command.verify_checksums,
            out_path: Some(command.out_path),
            md_path: Some(command.md_path),
        })?,
        VerifyMode::Cross => run_cross_verify(&CrossVerifyOptions {
            dir: command.dir,
            source_db: command
                .source
                .ok_or_else(|| AppError::invalid_argument("--source is required for cross mode"))?,
            sample_size: command.sample_size,
            max_failures: command.max_failures,
            verify_checksums: command.verify_checksums,
            out_path: Some(command.out_path),
            md_path: Some(command.md_path),
        })?,
    };
    print_verify_summary(&report);
    if report.has_failures() {
        return Err(AppError::new("VERIFY_FAILED", "verification failed"));
    }
    Ok(())
}

fn print_verify_summary(report: &RangeStrataVerifyReport) {
    println!("Range Strata Binary verification complete.");
    println!("  Dimensions: {}", report.totals.dimensions);
    println!("  Manifest OK: {}", report.totals.manifest_ok);
    println!("  Catalog OK: {}", report.totals.catalog_ok);
    println!(
        "  Index files OK: {}/{}",
        report.totals.index_files_ok,
        report.totals.index_files_ok + report.totals.index_files_failed
    );
    println!(
        "  Pack files OK: {}/{}",
        report.totals.pack_files_ok,
        report.totals.pack_files_ok + report.totals.pack_files_failed
    );
    println!(
        "  Index-Pack cross failures: {}",
        report.totals.index_pack_cross_failures
    );
    if report.mode == VerifyMode::Cross {
        println!(
            "  Source records checked: {}",
            report.totals.checked_source_records.unwrap_or_default()
        );
        println!(
            "  Source records failed: {}",
            report.totals.failed_source_records.unwrap_or_default()
        );
        println!(
            "  Extra binary records: {}",
            report.totals.extra_binary_records.unwrap_or_default()
        );
    }
    println!("  Total failures: {}", report.failures.len());
}

fn run_query(args: Vec<String>) -> Result<(), AppError> {
    let mut data_dir = None;
    let mut strategy = "default".to_owned();
    let mut player_count = None;
    let mut depth_bb = None;
    let mut concrete_line_id = None;
    let mut hole_cards = None;
    let mut verify_checksum = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
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
            "--hole-cards" => hole_cards = Some(next_value(&args, &mut index)?.to_owned()),
            "--verify-checksum" => verify_checksum = true,
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown query option: {option}"
                )))
            }
        }
        index += 1;
    }
    let service = QueryService::open(
        data_dir.ok_or_else(|| AppError::invalid_argument("--data-dir is required"))?,
        3,
        verify_checksum,
    )?;
    let result = service.query(
        &DimensionRef::new(
            strategy,
            player_count.ok_or_else(|| AppError::invalid_argument("--player-count is required"))?,
            depth_bb.ok_or_else(|| AppError::invalid_argument("--depth-bb is required"))?,
        ),
        concrete_line_id
            .ok_or_else(|| AppError::invalid_argument("--concrete-line-id is required"))?,
        &hole_cards.ok_or_else(|| AppError::invalid_argument("--hole-cards is required"))?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&result)
            .map_err(|error| AppError::invalid_format(error.to_string()))?
    );
    Ok(())
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}

fn parse_u32(name: &str, value: &str) -> Result<u32, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}

fn print_help() {
    println!(
        "poker-hands-storage-service

Commands:
  build --source-db <range.db> --out-dir <dir>
        [--dimension strategy:player_count:depth_bb]
        [--max-concrete-lines <count>] [--overwrite]

  query --data-dir <dir> --player-count <count> --depth-bb <bb>
        --concrete-line-id <id> --hole-cards <AA|AKs|AsKh>
        [--strategy <name>] [--verify-checksum]

  verify --mode <standalone|cross> --dir <dir>
        [--source <range.db>] [--sample-size <count>]
        [--max-failures <count>] [--verify-checksum]
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

  healthcheck [--url <http-url>] [--timeout-ms <milliseconds>]

  serve
        Environment: PHS_BIND, PHS_DATA_DIR, PHS_META_DB,
        PHS_MAX_OPEN_HANDLES, PHS_VERIFY_CHECKSUMS, PHS_PREWARM, RUST_LOG"
    );
}
