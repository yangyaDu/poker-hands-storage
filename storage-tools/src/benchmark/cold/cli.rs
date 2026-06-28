use std::path::PathBuf;

use crate::benchmark::cli::{
    next_value, parse_requested_dimension, parse_u32, parse_u64, parse_usize,
};
use crate::errors::ToolError;

use super::cache_eviction::default_filler_size;
use super::types::{
    BenchmarkColdCommand, BenchmarkColdCompareCommand, BenchmarkSqliteColdCommand, ColdStartMode,
    QueryPolicy,
};

// ── benchmark-cold ──────────────────────────────────────────────────

pub fn parse_benchmark_cold_args(args: Vec<String>) -> Result<BenchmarkColdCommand, ToolError> {
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
                    .map_err(ToolError::invalid_argument)?
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
                    .map_err(ToolError::invalid_argument)?
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
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-cold option: {option}"
                )))
            }
        }
        index += 1;
    }

    let dir = dir.ok_or_else(|| ToolError::invalid_argument("--dir is required"))?;
    let source = source.ok_or_else(|| ToolError::invalid_argument("--source is required"))?;
    let meta = meta.unwrap_or_else(|| dir.join("meta.db"));

    if runs_per_dimension == 0 {
        return Err(ToolError::invalid_argument(
            "--runs must be a positive integer",
        ));
    }

    // Default cache filler MB based on platform if not specified.
    let cache_filler_mb = cache_filler_mb.unwrap_or_else(|| {
        let dataset_size = super::cache_eviction::compute_dataset_size(&dir);
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

// ── benchmark-cold-compare ──────────────────────────────────────────

pub fn parse_benchmark_cold_compare_args(
    args: Vec<String>,
) -> Result<BenchmarkColdCompareCommand, ToolError> {
    let mut binary_report = None;
    let mut sqlite_report = None;
    let mut out_path = PathBuf::from("reports/benchmark-cold-compare.json");
    let mut md_path = PathBuf::from("reports/benchmark-cold-compare.md");
    let mut allow_mismatch = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--binary" => binary_report = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--sqlite" => sqlite_report = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            "--allow-mismatch" => allow_mismatch = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-cold-compare option: {option}"
                )))
            }
        }
        index += 1;
    }

    Ok(BenchmarkColdCompareCommand {
        binary_report: binary_report
            .ok_or_else(|| ToolError::invalid_argument("--binary is required"))?,
        sqlite_report: sqlite_report
            .ok_or_else(|| ToolError::invalid_argument("--sqlite is required"))?,
        out_path,
        md_path,
        allow_mismatch,
    })
}

// ── benchmark-sqlite-cold ───────────────────────────────────────────

pub fn parse_benchmark_sqlite_cold_args(
    args: Vec<String>,
) -> Result<BenchmarkSqliteColdCommand, ToolError> {
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
                    .map_err(ToolError::invalid_argument)?
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
                    .map_err(ToolError::invalid_argument)?
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
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-sqlite-cold option: {option}"
                )))
            }
        }
        index += 1;
    }

    let source = source.ok_or_else(|| ToolError::invalid_argument("--source is required"))?;
    let dir = dir.ok_or_else(|| ToolError::invalid_argument("--dir is required"))?;

    if runs_per_dimension == 0 {
        return Err(ToolError::invalid_argument(
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

// ── tests ───────────────────────────────────────────────────────────

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
