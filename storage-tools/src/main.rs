use std::path::PathBuf;

use poker_hands_storage_tools::errors::ToolError;
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
        Some("verify") => run_verify(args.collect()),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(cmd) => Err(ToolError::invalid_argument(format!(
            "Unknown command: {cmd}"
        ))),
    }
}

fn run_build(args: Vec<String>) -> Result<(), ToolError> {
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
                    ToolError::invalid_argument("--max-concrete-lines must be an integer")
                })?)
            }
            "--overwrite" => overwrite = true,
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

fn print_verify_summary(report: &RangeStrataVerifyReport) {
    println!("mode={:?}", report.mode);
    println!("pass={}", report.failures.is_empty());
    println!("failures={}", report.failures.len());
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, ToolError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| ToolError::invalid_argument("Missing option value"))
}

fn print_help() {
    println!(
        "poker-hands-storage-tools

Commands:
  build --source-db <range.db> --out-dir <dir>
        [--dimension strategy:player_count:depth_bb]
        [--max-concrete-lines <count>] [--overwrite]

  verify --dir <dir> [--mode standalone|cross] [--source <range.db>]
         [--verify-checksum] [--sample-size <n>] [--max-failures <n>]
         [--out <report.json>] [--md <report.md>]

  help  Print this help message"
    );
}
