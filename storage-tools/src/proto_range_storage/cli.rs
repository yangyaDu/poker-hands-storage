use range_store_core::dimension::DimensionSpec;
use range_store_core::hole_cards::hand_code_from_id;
use std::path::PathBuf;

use crate::benchmark::cli::{next_value, parse_u32, parse_u64, parse_usize};
use crate::benchmark::cold::types::ColdStartMode;
use crate::errors::ToolError;

use super::line_matrix_store::{
    CompactLineMatrixArchiveOptions, CompactLineMatrixArchivesOptions,
    CompactVsCoreBenchmarkCommand, CompactVsCoreColdWorkerCommand, CompactVsCoreEngine,
    CompactVsCoreQuery,
};

pub fn parse_export_compact_line_matrix_archive_args(
    args: Vec<String>,
) -> Result<CompactLineMatrixArchiveOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut dimension = DimensionSpec::parse("default:6:100")?;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => dimension = DimensionSpec::parse(next_value(&args, &mut index)?)?,
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown export-compact-line-matrix-archive option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(CompactLineMatrixArchiveOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        dimension,
        overwrite,
    })
}

pub fn parse_verify_compact_line_matrix_archive_args(
    args: Vec<String>,
) -> Result<PathBuf, ToolError> {
    let mut dir = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown verify-compact-line-matrix-archive option: {option}"
                )))
            }
        }
        index += 1;
    }
    dir.ok_or_else(|| ToolError::invalid_argument("--dir is required"))
}

pub fn parse_export_all_compact_line_matrix_archives_args(
    args: Vec<String>,
) -> Result<CompactLineMatrixArchivesOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown export-all-compact-line-matrix-archives option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(CompactLineMatrixArchivesOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        overwrite,
    })
}

pub fn parse_benchmark_compact_vs_core_args(
    args: Vec<String>,
) -> Result<CompactVsCoreBenchmarkCommand, ToolError> {
    let mut compact_dir = None;
    let mut core_dir = None;
    let mut dimension = None;
    let mut hot_iterations = 1_000usize;
    let mut warmup_iterations = 100usize;
    let mut cold_runs = 20usize;
    let mut cold_mode = ColdStartMode::ProcessCold;
    let mut cache_filler_mb = None;
    let mut seed = 42u64;
    let mut max_open_handles = 2usize;
    let mut verify_checksums = false;
    let mut concrete_line_id = None;
    let mut hand_id = None;
    let mut out_path = PathBuf::from("reports/benchmark-compact-vs-core.json");
    let mut md_path = PathBuf::from("reports/benchmark-compact-vs-core.md");

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--compact-dir" => compact_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--core-dir" => core_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--hot-iterations" => {
                hot_iterations = parse_usize("--hot-iterations", next_value(&args, &mut index)?)?
            }
            "--warmup-iterations" => {
                warmup_iterations =
                    parse_usize("--warmup-iterations", next_value(&args, &mut index)?)?
            }
            "--cold-runs" => {
                cold_runs = parse_usize("--cold-runs", next_value(&args, &mut index)?)?
            }
            "--cold-mode" => {
                cold_mode = ColdStartMode::parse(next_value(&args, &mut index)?)
                    .map_err(ToolError::invalid_argument)?
            }
            "--cache-filler-mb" => {
                cache_filler_mb = Some(parse_u64(
                    "--cache-filler-mb",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--seed" => seed = parse_u64("--seed", next_value(&args, &mut index)?)?,
            "--max-open-handles" => {
                max_open_handles =
                    parse_usize("--max-open-handles", next_value(&args, &mut index)?)?
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u64(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand-id" => hand_id = Some(parse_u32("--hand-id", next_value(&args, &mut index)?)?),
            "--verify-checksum" => verify_checksums = true,
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-compact-vs-core option: {option}"
                )))
            }
        }
        index += 1;
    }

    let fixed_query = build_fixed_query(concrete_line_id, hand_id)?;
    Ok(CompactVsCoreBenchmarkCommand {
        compact_dir: compact_dir
            .ok_or_else(|| ToolError::invalid_argument("--compact-dir is required"))?,
        core_dir: core_dir.ok_or_else(|| ToolError::invalid_argument("--core-dir is required"))?,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        hot_iterations: require_positive("--hot-iterations", hot_iterations)?,
        warmup_iterations,
        cold_runs: require_positive("--cold-runs", cold_runs)?,
        cold_mode,
        cache_filler_mb,
        seed,
        max_open_handles: require_positive("--max-open-handles", max_open_handles)?,
        verify_checksums,
        fixed_query,
        out_path,
        md_path,
    })
}

pub fn parse_compact_vs_core_cold_worker_args(
    args: Vec<String>,
) -> Result<CompactVsCoreColdWorkerCommand, ToolError> {
    let mut engine = None;
    let mut compact_dir = None;
    let mut core_dir = None;
    let mut dimension = None;
    let mut concrete_line_id = None;
    let mut hand_id = None;
    let mut max_open_handles = 2usize;
    let mut verify_checksums = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--engine" => {
                engine = Some(CompactVsCoreEngine::parse(next_value(&args, &mut index)?)?)
            }
            "--compact-dir" => compact_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--core-dir" => core_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u64(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand-id" => hand_id = Some(parse_u32("--hand-id", next_value(&args, &mut index)?)?),
            "--max-open-handles" => {
                max_open_handles =
                    parse_usize("--max-open-handles", next_value(&args, &mut index)?)?
            }
            "--verify-checksum" => verify_checksums = true,
            "--no-verify-checksum" => verify_checksums = false,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown compact-vs-core-cold-worker option: {option}"
                )))
            }
        }
        index += 1;
    }

    let query = build_fixed_query(concrete_line_id, hand_id)?.ok_or_else(|| {
        ToolError::invalid_argument("--concrete-line-id and --hand-id are required")
    })?;
    Ok(CompactVsCoreColdWorkerCommand {
        engine: engine.ok_or_else(|| ToolError::invalid_argument("--engine is required"))?,
        compact_dir: compact_dir
            .ok_or_else(|| ToolError::invalid_argument("--compact-dir is required"))?,
        core_dir: core_dir.ok_or_else(|| ToolError::invalid_argument("--core-dir is required"))?,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        query,
        max_open_handles: require_positive("--max-open-handles", max_open_handles)?,
        verify_checksums,
    })
}

fn build_fixed_query(
    concrete_line_id: Option<u64>,
    hand_id: Option<u32>,
) -> Result<Option<CompactVsCoreQuery>, ToolError> {
    match (concrete_line_id, hand_id) {
        (None, None) => Ok(None),
        (Some(concrete_line_id), Some(hand_id)) if hand_id < 169 => {
            let hand_id = hand_id as u8;
            Ok(Some(CompactVsCoreQuery {
                concrete_line_id,
                hand_id,
                hand: hand_code_from_id(hand_id),
            }))
        }
        (Some(_), Some(_)) => Err(ToolError::invalid_argument("--hand-id must be in 0..=168")),
        _ => Err(ToolError::invalid_argument(
            "--concrete-line-id and --hand-id must be provided together",
        )),
    }
}

fn require_positive(name: &str, value: usize) -> Result<usize, ToolError> {
    if value == 0 {
        Err(ToolError::invalid_argument(format!(
            "{name} must be at least 1"
        )))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_vs_core_defaults() {
        let command = parse_benchmark_compact_vs_core_args(vec![
            "--compact-dir".to_owned(),
            "compact".to_owned(),
            "--core-dir".to_owned(),
            "core".to_owned(),
            "--dimension".to_owned(),
            "default:9:200".to_owned(),
        ])
        .expect("parse command");

        assert_eq!(command.hot_iterations, 1_000);
        assert_eq!(command.cold_runs, 20);
        assert_eq!(command.dimension.player_count, 9);
        assert!(command.fixed_query.is_none());
    }

    #[test]
    fn rejects_partial_fixed_query() {
        let error = parse_benchmark_compact_vs_core_args(vec![
            "--compact-dir".to_owned(),
            "compact".to_owned(),
            "--core-dir".to_owned(),
            "core".to_owned(),
            "--dimension".to_owned(),
            "default:9:200".to_owned(),
            "--concrete-line-id".to_owned(),
            "1".to_owned(),
        ])
        .expect_err("partial fixed query must fail");

        assert_eq!(error.code(), "INVALID_ARGUMENT");
    }
}
