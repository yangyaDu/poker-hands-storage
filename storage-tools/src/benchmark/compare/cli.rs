use std::path::PathBuf;

use crate::benchmark::cli::next_value;
use crate::errors::ToolError;

use super::types::BenchmarkCompareCommand;

pub fn parse_benchmark_compare_args(
    args: Vec<String>,
) -> Result<BenchmarkCompareCommand, ToolError> {
    let mut binary_report = None;
    let mut sqlite_report = None;
    let mut out_path = PathBuf::from("reports/benchmark-compare.json");
    let mut md_path = PathBuf::from("reports/benchmark-compare.md");
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
                    "Unknown benchmark-compare option: {option}"
                )))
            }
        }
        index += 1;
    }

    Ok(BenchmarkCompareCommand {
        binary_report: binary_report
            .ok_or_else(|| ToolError::invalid_argument("--binary is required"))?,
        sqlite_report: sqlite_report
            .ok_or_else(|| ToolError::invalid_argument("--sqlite is required"))?,
        out_path,
        md_path,
        allow_mismatch,
    })
}
