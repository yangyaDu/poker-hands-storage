use std::path::PathBuf;

use crate::benchmark::compare::types::BenchmarkCompareCommand;
use crate::errors::AppError;

pub fn parse_benchmark_compare_args(
    args: Vec<String>,
) -> Result<BenchmarkCompareCommand, AppError> {
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
                return Err(AppError::invalid_argument(format!(
                    "Unknown benchmark-compare option: {option}"
                )))
            }
        }
        index += 1;
    }

    Ok(BenchmarkCompareCommand {
        binary_report: binary_report
            .ok_or_else(|| AppError::invalid_argument("--binary is required"))?,
        sqlite_report: sqlite_report
            .ok_or_else(|| AppError::invalid_argument("--sqlite is required"))?,
        out_path,
        md_path,
        allow_mismatch,
    })
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}
