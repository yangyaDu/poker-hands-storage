use std::path::PathBuf;

use crate::benchmark::cli::{
    next_value, parse_requested_dimension, parse_u32, parse_usize, parse_usize_list,
};
use crate::benchmark::types::{normalize_batch_sizes, WorkloadMode};
use crate::errors::ToolError;

use super::types::BenchmarkNativeCommand;

pub fn parse_benchmark_native_args(args: Vec<String>) -> Result<BenchmarkNativeCommand, ToolError> {
    let mut source = None;
    let mut dir = None;
    let mut meta = None;
    let mut native_entry = PathBuf::from("range-store-native/index.js");
    let mut http_service_bin = None;
    let mut bun = PathBuf::from("bun");
    let mut max_open_handles = 2_u32;
    let mut out_path = PathBuf::from("reports/benchmark-bun-native.json");
    let mut md_path = PathBuf::from("reports/benchmark-bun-native.md");
    let mut workload_path = None;
    let mut write_workload_path = None;
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
    let mut verify_checksums = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--meta" => meta = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--native-entry" => native_entry = PathBuf::from(next_value(&args, &mut index)?),
            "--http-service-bin" => {
                http_service_bin = Some(PathBuf::from(next_value(&args, &mut index)?))
            }
            "--bun" => bun = PathBuf::from(next_value(&args, &mut index)?),
            "--max-open-handles" => {
                max_open_handles =
                    parse_u32("--max-open-handles", next_value(&args, &mut index)?)?.max(1)
            }
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            "--workload" => workload_path = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--write-workload" => {
                write_workload_path = Some(PathBuf::from(next_value(&args, &mut index)?))
            }
            "--seed" => {
                seed = next_value(&args, &mut index)?
                    .parse()
                    .map_err(|_| ToolError::invalid_argument("--seed must be an integer"))?
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
            "--verify-checksum" => verify_checksums = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-native option: {option}"
                )))
            }
        }
        index += 1;
    }

    let dir = dir.ok_or_else(|| ToolError::invalid_argument("--dir is required"))?;
    let source = source.ok_or_else(|| ToolError::invalid_argument("--source is required"))?;
    let meta = meta.unwrap_or_else(|| dir.join("meta.db"));
    let hand_iterations = hand_iterations.unwrap_or(iterations);
    let batch_iterations = batch_iterations.unwrap_or(iterations.min(200));
    let batch_sizes = normalize_batch_sizes(batch_size, &batch_sizes);
    if workload_path.is_some() && write_workload_path.is_some() {
        return Err(ToolError::invalid_argument(
            "--workload and --write-workload cannot be used together",
        ));
    }

    Ok(BenchmarkNativeCommand {
        source,
        dir,
        meta,
        native_entry,
        http_service_bin,
        bun,
        max_open_handles,
        out_path,
        md_path,
        workload_path,
        write_workload_path,
        seed,
        hand_iterations,
        batch_iterations,
        batch_size,
        batch_sizes,
        requested_dimensions,
        requested_dimension_values,
        workload_mode,
        warmup_iterations,
        verify_checksums,
    })
}
