use std::path::PathBuf;

use crate::benchmark::cold::cache_eviction::default_filler_size;
use crate::benchmark::cold::types::{BenchmarkSqliteColdCommand, ColdStartMode, QueryPolicy};
use crate::errors::AppError;
use crate::scripts::benchmark::parse_requested_dimension;

pub fn parse_benchmark_sqlite_cold_args(
    args: Vec<String>,
) -> Result<BenchmarkSqliteColdCommand, AppError> {
    let mut source = None;
    let mut dir = None;
    let mut out_path = PathBuf::from("reports/benchmark-sqlite-cold-start.json");
    let mut md_path = PathBuf::from("reports/benchmark-sqlite-cold-start.md");
    let mut mode = ColdStartMode::ProcessCold;
    let mut runs_per_dimension = 10_usize;
    let mut requested_dimensions = Vec::new();
    let mut query_policy = QueryPolicy::First;
    let mut fixed_concrete_line_id = None;
    let mut fixed_hand = None;
    let mut cache_filler_mb = None;
    let mut max_errors_per_dimension = usize::MAX;
    let mut fail_fast = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            "--mode" => {
                mode = ColdStartMode::parse(next_value(&args, &mut index)?)
                    .map_err(AppError::invalid_argument)?
            }
            "--runs" | "--runs-per-dimension" => {
                runs_per_dimension = parse_usize("--runs", next_value(&args, &mut index)?)?
            }
            "--dimension" => {
                let value = next_value(&args, &mut index)?;
                requested_dimensions.push(parse_requested_dimension(value)?);
            }
            "--query-policy" => {
                query_policy = QueryPolicy::parse(next_value(&args, &mut index)?)
                    .map_err(AppError::invalid_argument)?
            }
            "--concrete-line-id" => {
                fixed_concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand" => fixed_hand = Some(next_value(&args, &mut index)?.to_owned()),
            "--cache-filler-mb" => {
                cache_filler_mb = Some(parse_u64(
                    "--cache-filler-mb",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--max-errors-per-dimension" => {
                max_errors_per_dimension =
                    parse_usize("--max-errors-per-dimension", next_value(&args, &mut index)?)?
            }
            "--fail-fast" => fail_fast = true,
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown benchmark-sqlite-cold option: {option}"
                )))
            }
        }
        index += 1;
    }

    let source = source.ok_or_else(|| AppError::invalid_argument("--source is required"))?;
    let dir = dir.ok_or_else(|| AppError::invalid_argument("--dir is required"))?;

    if runs_per_dimension == 0 {
        return Err(AppError::invalid_argument(
            "--runs must be a positive integer",
        ));
    }

    let cache_filler_mb = cache_filler_mb.unwrap_or_else(|| {
        let source_size = std::fs::metadata(&source)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        default_filler_size(source_size) / (1024 * 1024)
    });

    Ok(BenchmarkSqliteColdCommand {
        source,
        dir,
        out_path,
        md_path,
        mode,
        runs_per_dimension,
        requested_dimensions,
        query_policy,
        fixed_concrete_line_id,
        fixed_hand,
        cache_filler_mb,
        max_errors_per_dimension,
        fail_fast,
    })
}

fn parse_usize(name: &str, value: &str) -> Result<usize, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}

fn parse_u32(name: &str, value: &str) -> Result<u32, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}

fn parse_u64(name: &str, value: &str) -> Result<u64, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}
