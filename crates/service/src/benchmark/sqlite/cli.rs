use std::path::PathBuf;

use crate::benchmark::cli::{next_value, parse_requested_dimension, parse_usize, parse_usize_list};
use crate::benchmark::types::{normalize_batch_sizes, WorkloadMode};
use crate::errors::AppError;

use super::types::BenchmarkSqliteCommand;

pub fn parse_benchmark_sqlite_args(args: Vec<String>) -> Result<BenchmarkSqliteCommand, AppError> {
    let mut source = None;
    let mut out_path = PathBuf::from("reports/benchmark-sqlite.json");
    let mut md_path = PathBuf::from("reports/benchmark-sqlite.md");
    let mut workload_path = None;
    let mut seed = 42_u64;
    let mut iterations = 1000_usize;
    let mut hand_iterations = None;
    let mut batch_iterations = None;
    let mut batch_size = 20_usize;
    let mut batch_sizes = vec![1, 5, 10, 50, 100];
    let mut requested_dimensions = Vec::new();
    let mut requested_dimension_values = Vec::new();
    let mut workload_mode = WorkloadMode::Random;
    let mut warmup_iterations = 20_usize;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            "--workload" => workload_path = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--seed" => {
                seed = next_value(&args, &mut index)?
                    .parse()
                    .map_err(|_| AppError::invalid_argument("--seed must be an integer"))?
            }
            "--iterations" => {
                iterations = parse_usize("--iterations", next_value(&args, &mut index)?)?
            }
            "--hand-iterations" => {
                hand_iterations = Some(parse_usize(
                    "--hand-iterations",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--batch-iterations" => {
                batch_iterations = Some(parse_usize(
                    "--batch-iterations",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--batch-size" => {
                batch_size = parse_usize("--batch-size", next_value(&args, &mut index)?)?.max(1)
            }
            "--batch-sizes" => {
                batch_sizes = parse_usize_list("--batch-sizes", next_value(&args, &mut index)?)?
            }
            "--dimension" => {
                let value = next_value(&args, &mut index)?.to_owned();
                requested_dimensions.push(parse_requested_dimension(&value)?);
                requested_dimension_values.push(value);
            }
            "--workload-mode" => {
                workload_mode = WorkloadMode::parse(next_value(&args, &mut index)?)?
            }
            "--warmup-iterations" => {
                warmup_iterations =
                    parse_usize("--warmup-iterations", next_value(&args, &mut index)?)?
            }
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown benchmark-sqlite option: {option}"
                )))
            }
        }
        index += 1;
    }

    let source = source.ok_or_else(|| AppError::invalid_argument("--source is required"))?;
    let hand_iterations = hand_iterations.unwrap_or(iterations);
    let batch_iterations = batch_iterations.unwrap_or(iterations.min(200));
    let batch_sizes = normalize_batch_sizes(batch_size, &batch_sizes);

    Ok(BenchmarkSqliteCommand {
        source,
        out_path,
        md_path,
        workload_path,
        seed,
        hand_iterations,
        batch_iterations,
        batch_size,
        batch_sizes,
        requested_dimensions,
        requested_dimension_values,
        workload_mode,
        warmup_iterations,
    })
}
