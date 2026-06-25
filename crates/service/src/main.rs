use std::env;
use std::path::PathBuf;

use poker_hands_storage_service::builder::{build_store, BuildOptions, DimensionSpec};
use poker_hands_storage_service::error::AppError;
use poker_hands_storage_service::naming::DimensionRef;
use poker_hands_storage_service::query_service::QueryService;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("build") => run_build(args.collect()),
        Some("query") => run_query(args.collect()),
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(AppError::invalid_argument(format!(
            "Unknown command: {command}"
        ))),
    }
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
        [--strategy <name>] [--verify-checksum]"
    );
}
