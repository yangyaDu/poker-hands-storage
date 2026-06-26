use std::path::PathBuf;

use crate::benchmark::cache_eviction::default_filler_size;
use crate::benchmark::cold_types::{BenchmarkColdCommand, ColdStartMode, QueryPolicy};
use crate::errors::AppError;
use crate::scripts::benchmark::parse_requested_dimension;

pub fn parse_benchmark_cold_args(args: Vec<String>) -> Result<BenchmarkColdCommand, AppError> {
    let mut source = None;
    let mut dir = None;
    let mut meta = None;
    let mut out_path = PathBuf::from("reports/benchmark-cold-start.json");
    let mut md_path = PathBuf::from("reports/benchmark-cold-start.md");
    let mut mode = ColdStartMode::ProcessCold;
    let mut runs_per_dimension = 10_usize;
    let mut requested_dimensions = Vec::new();
    let mut query_policy = QueryPolicy::First;
    let mut fixed_concrete_line_id = None;
    let mut fixed_hand = None;
    let mut cache_filler_mb = None;
    let mut max_errors_per_dimension = usize::MAX;
    let mut fail_fast = false;
    let mut verify_checksums = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--meta" => meta = Some(PathBuf::from(next_value(&args, &mut index)?)),
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
            "--verify-checksum" => verify_checksums = true,
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown benchmark-cold option: {option}"
                )))
            }
        }
        index += 1;
    }

    let dir = dir.ok_or_else(|| AppError::invalid_argument("--dir is required"))?;
    let source = source.ok_or_else(|| AppError::invalid_argument("--source is required"))?;
    let meta = meta.unwrap_or_else(|| dir.join("meta.db"));

    if runs_per_dimension == 0 {
        return Err(AppError::invalid_argument(
            "--runs must be a positive integer",
        ));
    }

    // Default cache filler MB based on platform if not specified.
    let cache_filler_mb = cache_filler_mb.unwrap_or_else(|| {
        let dataset_size = crate::benchmark::cache_eviction::compute_dataset_size(&dir);
        default_filler_size(dataset_size) / (1024 * 1024)
    });

    Ok(BenchmarkColdCommand {
        source,
        dir,
        meta,
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
        verify_checksums,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults() {
        let cmd =
            parse_benchmark_cold_args(args(&["--dir", "data/test", "--source", "data/test.db"]))
                .unwrap();
        assert_eq!(cmd.mode, ColdStartMode::ProcessCold);
        assert_eq!(cmd.runs_per_dimension, 10);
        assert_eq!(cmd.query_policy, QueryPolicy::First);
        assert!(!cmd.fail_fast);
        assert!(!cmd.verify_checksums);
    }

    #[test]
    fn mode_parse() {
        let cmd = parse_benchmark_cold_args(args(&[
            "--dir",
            "d",
            "--source",
            "s",
            "--mode",
            "os-best-effort",
        ]))
        .unwrap();
        assert_eq!(cmd.mode, ColdStartMode::OsBestEffort);
    }

    #[test]
    fn invalid_mode() {
        let result =
            parse_benchmark_cold_args(args(&["--dir", "d", "--source", "s", "--mode", "invalid"]));
        assert!(result.is_err());
    }

    #[test]
    fn runs_zero_rejected() {
        let result =
            parse_benchmark_cold_args(args(&["--dir", "d", "--source", "s", "--runs", "0"]));
        assert!(result.is_err());
    }

    #[test]
    fn missing_dir() {
        let result = parse_benchmark_cold_args(args(&["--source", "s"]));
        assert!(result.is_err());
    }
}
