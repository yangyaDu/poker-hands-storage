use std::path::PathBuf;

use range_store_core::dimension::DimensionSpec;

use crate::benchmark::cli::{next_value, parse_usize};
use crate::errors::ToolError;

use super::archive::{V3ArchiveExportOptions, V3ArchivesExportOptions};
use super::benchmark::V3BenchmarkCommand;
use super::metadata_store::DEFAULT_METADATA_PAGE_TARGET_BYTES;
use super::verification::V3VerificationOptions;

#[derive(Debug, Clone)]
pub struct V3VerifyCommand {
    pub archive_dir: PathBuf,
    pub source_db: Option<PathBuf>,
    pub out_path: Option<PathBuf>,
    pub options: V3VerificationOptions,
}

pub fn parse_v3_export_args(args: Vec<String>) -> Result<V3ArchiveExportOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut dimension = None;
    let mut page_target_bytes = DEFAULT_METADATA_PAGE_TARGET_BYTES;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--page-target-bytes" => {
                page_target_bytes =
                    parse_usize("--page-target-bytes", next_value(&args, &mut index)?)?
            }
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown v3-export option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(V3ArchiveExportOptions {
        source_db: required_path(source_db, "--source")?,
        out_dir: required_path(out_dir, "--out")?,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        metadata_page_target_bytes: page_target_bytes,
        overwrite,
    })
}

pub fn parse_v3_export_all_args(args: Vec<String>) -> Result<V3ArchivesExportOptions, ToolError> {
    let mut source_db = None;
    let mut out_root = None;
    let mut page_target_bytes = DEFAULT_METADATA_PAGE_TARGET_BYTES;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-root" => out_root = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--page-target-bytes" => {
                page_target_bytes =
                    parse_usize("--page-target-bytes", next_value(&args, &mut index)?)?
            }
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown v3-export-all option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(V3ArchivesExportOptions {
        source_db: required_path(source_db, "--source")?,
        out_root: required_path(out_root, "--out-root")?,
        metadata_page_target_bytes: page_target_bytes,
        overwrite,
    })
}

pub fn parse_v3_verify_args(
    args: Vec<String>,
    requires_source: bool,
) -> Result<V3VerifyCommand, ToolError> {
    let mut archive_dir = None;
    let mut source_db = None;
    let mut out_path = None;
    let mut max_failure_samples = V3VerificationOptions::default().max_failure_samples;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--archive" => archive_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--source" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out" => out_path = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--max-failure-samples" => {
                max_failure_samples =
                    parse_usize("--max-failure-samples", next_value(&args, &mut index)?)?
            }
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown V3 verification option: {option}"
                )))
            }
        }
        index += 1;
    }
    if requires_source && source_db.is_none() {
        return Err(ToolError::invalid_argument("--source is required"));
    }
    Ok(V3VerifyCommand {
        archive_dir: required_path(archive_dir, "--archive")?,
        source_db,
        out_path,
        options: V3VerificationOptions {
            max_failure_samples,
        },
    })
}

pub fn parse_v3_benchmark_args(args: Vec<String>) -> Result<V3BenchmarkCommand, ToolError> {
    let mut source_db = None;
    let mut archive_root = None;
    let mut dimension = None;
    let mut iterations = 1_000;
    let mut warmup_iterations = 50;
    let mut max_open_handles = 2;
    let mut metadata_cache_byte_budget_per_handle = 8 * 1024 * 1024;
    let mut strategy_cache_byte_budget_per_handle = 64 * 1024 * 1024;
    let mut verify_file_checksums = false;
    let mut out_path = PathBuf::from("reports/v3-benchmark.json");
    let mut markdown_path = PathBuf::from("reports/v3-benchmark.md");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--archive-root" => archive_root = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--iterations" => {
                iterations = parse_usize("--iterations", next_value(&args, &mut index)?)?
            }
            "--warmup-iterations" => {
                warmup_iterations =
                    parse_usize("--warmup-iterations", next_value(&args, &mut index)?)?
            }
            "--max-open-handles" => {
                max_open_handles =
                    parse_usize("--max-open-handles", next_value(&args, &mut index)?)?
            }
            "--metadata-cache-bytes" => {
                metadata_cache_byte_budget_per_handle =
                    parse_usize("--metadata-cache-bytes", next_value(&args, &mut index)?)?
            }
            "--strategy-cache-bytes" => {
                strategy_cache_byte_budget_per_handle =
                    parse_usize("--strategy-cache-bytes", next_value(&args, &mut index)?)?
            }
            "--verify-checksums" => verify_file_checksums = true,
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => markdown_path = PathBuf::from(next_value(&args, &mut index)?),
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown v3-benchmark option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(V3BenchmarkCommand {
        source_db: required_path(source_db, "--source")?,
        archive_root: required_path(archive_root, "--archive-root")?,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        iterations,
        warmup_iterations,
        max_open_handles,
        metadata_cache_byte_budget_per_handle,
        strategy_cache_byte_budget_per_handle,
        verify_file_checksums,
        out_path,
        markdown_path,
    })
}

fn required_path(value: Option<PathBuf>, name: &str) -> Result<PathBuf, ToolError> {
    value.ok_or_else(|| ToolError::invalid_argument(format!("{name} is required")))
}
