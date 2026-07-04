use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::benchmark::memory_snapshot::{BenchmarkMemoryReport, MemorySnapshot};
use crate::benchmark::metrics::{build_totals, BenchmarkCaseResult};
use crate::benchmark::report::{
    build_benchmark_report_for_engine, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::types::{
    BenchmarkWorkload, HandsByActionsBenchmarkItem, WorkloadOptions, WorkloadSource,
};
use crate::benchmark::workload::{
    create_benchmark_workload, read_workload_json, write_workload_json,
};
use crate::errors::ToolError;
use range_store_core::dimension::{get_concrete_lines_table_name, quote_identifier, DimensionRef};
use range_store_core::sqlite::{Connection, Value};

use super::types::BenchmarkNativeCommand;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeWorkerInput {
    data_dir: String,
    native_entry: String,
    native_node_entry: String,
    max_open_handles: u32,
    verify_checksums: bool,
    warmup_iterations: usize,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConcreteLineLookupQuery {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line_id: u32,
    concrete_line: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LineToHandsByActionsQuery {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line_id: u32,
    concrete_line: String,
    actions: Vec<String>,
    frequency: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeWorkerOutput {
    cold_start: serde_json::Value,
    cases: Vec<BenchmarkCaseResult>,
    memory_before: MemorySnapshot,
    memory_after: MemorySnapshot,
    notes: Vec<String>,
}

pub fn run_native_benchmark(
    command: &BenchmarkNativeCommand,
) -> Result<BenchmarkRunReport, ToolError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_mode = workload.mode;
    let meta_connection = Connection::open(&command.meta, true)?;
    let concrete_line_queries = build_concrete_line_queries(&meta_connection, &workload)?;
    let line_to_hands_by_actions_queries =
        build_line_to_hands_by_actions_queries(&meta_connection, &workload)?;
    let worker_output = run_native_worker(
        command,
        workload.clone(),
        concrete_line_queries,
        line_to_hands_by_actions_queries,
    )?;

    let memory =
        BenchmarkMemoryReport::new(worker_output.memory_before, worker_output.memory_after);
    let totals = build_totals(&worker_output.cases);
    let mut notes = vec![
        "Bun native benchmark; storage-tools orchestrates workload/reporting and Bun loads range-store-native as the measured production entrypoint.".to_owned(),
        "Cold start is measured inside the Bun worker as dynamic import + PokerHandsRange construction + first hand query.".to_owned(),
        "Result counts are case-specific: concrete line lookups, action entries, batch action entries, and matching hands.".to_owned(),
        "Native SDK exposes metadata lookup APIs, but this command currently does not measure the drill-scenarios case.".to_owned(),
        "Exact concrete-line lookup cases skip empty concrete_line rows because the business API treats empty strings as invalid input.".to_owned(),
        "Use the same workload JSON when comparing against SQLite, Rust core direct, or HTTP service reports.".to_owned(),
    ];
    notes.extend(worker_output.notes);

    let mut report = build_benchmark_report_for_engine(
        ReportInput {
            source_db_path: command.source.display().to_string(),
            binary_dir: command.dir.display().to_string(),
            meta_db_path: command.meta.display().to_string(),
            options: BenchmarkOptionsSummary {
                seed: command.seed,
                requested_dimensions: command.requested_dimension_values.clone(),
                hand_iterations: command.hand_iterations,
                batch_iterations: command.batch_iterations,
                batch_size: command.batch_size,
                batch_sizes: command.batch_sizes.clone(),
                warmup_iterations: command.warmup_iterations,
                verify_checksums: command.verify_checksums,
                verify_results: false,
                workload_mode,
            },
            workload,
            workload_source,
            workload_path: benchmark_workload_path(command),
            cases: worker_output.cases,
            totals,
            memory,
            result_verification: None,
            notes,
        },
        "bun-native",
    );
    report.cold_start = Some(worker_output.cold_start);

    write_benchmark_json(&command.out_path, &report)?;
    write_benchmark_markdown(&command.md_path, &report)?;
    Ok(report)
}

fn load_or_create_workload(
    command: &BenchmarkNativeCommand,
) -> Result<(BenchmarkWorkload, WorkloadSource), ToolError> {
    if let Some(path) = &command.workload_path {
        Ok((read_workload_json(path)?, WorkloadSource::Loaded))
    } else {
        let workload = create_benchmark_workload(&WorkloadOptions {
            source_db_path: command.source.clone(),
            requested_dimensions: command.requested_dimensions.clone(),
            seed: command.seed,
            hand_iterations: command.hand_iterations,
            batch_iterations: command.batch_iterations,
            batch_size: command.batch_size,
            batch_sizes: command.batch_sizes.clone(),
            workload_mode: command.workload_mode,
        })?;
        if let Some(path) = &command.write_workload_path {
            write_workload_json(path, &workload)?;
        }
        Ok((workload, WorkloadSource::Generated))
    }
}

fn benchmark_workload_path(command: &BenchmarkNativeCommand) -> Option<String> {
    command
        .workload_path
        .as_ref()
        .or(command.write_workload_path.as_ref())
        .map(|path| path.display().to_string())
}

fn run_native_worker(
    command: &BenchmarkNativeCommand,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
) -> Result<NativeWorkerOutput, ToolError> {
    let input = NativeWorkerInput {
        data_dir: absolute_existing_path(&command.dir, "--dir")?
            .display()
            .to_string(),
        native_entry: absolute_existing_path(&command.native_entry, "--native-entry")?
            .display()
            .to_string(),
        native_node_entry: native_node_entry(&command.native_entry)?
            .display()
            .to_string(),
        max_open_handles: command.max_open_handles,
        verify_checksums: command.verify_checksums,
        warmup_iterations: command.warmup_iterations,
        workload,
        concrete_line_queries,
        line_to_hands_by_actions_queries,
    };
    let input_path = write_worker_input(&input)?;
    let worker_path = default_worker_path();
    let output = Command::new(&command.bun)
        .arg(&worker_path)
        .arg(&input_path)
        .output()
        .map_err(|error| {
            ToolError::new(
                "NATIVE_BENCHMARK_FAILED",
                format!(
                    "Failed to start Bun worker `{}`: {error}",
                    command.bun.display()
                ),
            )
        })?;
    let _ = fs::remove_file(&input_path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(ToolError::new(
            "NATIVE_BENCHMARK_FAILED",
            format!(
                "Bun native benchmark worker failed with status {:?}. stderr: {} stdout: {}",
                output.status.code(),
                stderr.trim(),
                stdout.trim()
            ),
        ));
    }
    serde_json::from_str::<NativeWorkerOutput>(&stdout).map_err(|error| {
        ToolError::invalid_format(format!(
            "Bun native worker returned invalid JSON: {error}. stdout: {} stderr: {}",
            stdout.trim(),
            stderr.trim()
        ))
    })
}

fn native_node_entry(native_entry: &Path) -> Result<PathBuf, ToolError> {
    let entry = if native_entry.is_absolute() {
        native_entry.to_path_buf()
    } else {
        std::env::current_dir()?.join(native_entry)
    };
    let node_entry = entry
        .parent()
        .ok_or_else(|| ToolError::invalid_argument("--native-entry must have a parent directory"))?
        .join("index.node");
    absolute_existing_path(&node_entry, "index.node")
}

fn write_worker_input(input: &NativeWorkerInput) -> Result<PathBuf, ToolError> {
    let path = std::env::temp_dir().join(format!(
        "poker-hands-native-benchmark-{}-{}.json",
        std::process::id(),
        crate::benchmark::report::generated_at_utc().replace([':', '.'], "-")
    ));
    let json = serde_json::to_string_pretty(input)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(&path, json)?;
    Ok(path)
}

fn default_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/benchmark/native/worker.mjs")
}

fn absolute_existing_path(path: &Path, option_name: &str) -> Result<PathBuf, ToolError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    if path.exists() {
        Ok(path)
    } else {
        Err(ToolError::invalid_argument(format!(
            "{option_name} path does not exist or is not accessible: {}",
            path.display()
        )))
    }
}

fn build_concrete_line_queries(
    connection: &Connection,
    workload: &BenchmarkWorkload,
) -> Result<Vec<ConcreteLineLookupQuery>, ToolError> {
    let mut cache = ConcreteLineCache::new(connection);
    let mut queries = Vec::new();
    for item in &workload.hand_queries {
        let concrete_line = cache.get(&item.dimension(), item.concrete_line_id)?;
        if concrete_line.trim().is_empty() {
            continue;
        }
        queries.push(ConcreteLineLookupQuery {
            strategy: item.strategy.clone(),
            player_count: item.player_count,
            depth_bb: item.depth_bb,
            concrete_line_id: item.concrete_line_id,
            concrete_line,
        });
    }
    Ok(queries)
}

fn build_line_to_hands_by_actions_queries(
    connection: &Connection,
    workload: &BenchmarkWorkload,
) -> Result<Vec<LineToHandsByActionsQuery>, ToolError> {
    let mut cache = ConcreteLineCache::new(connection);
    let mut queries = Vec::new();
    for item in &workload.hands_by_actions_queries {
        if let Some(query) = line_to_hands_by_actions_query(&mut cache, item)? {
            queries.push(query);
        }
    }
    Ok(queries)
}

fn line_to_hands_by_actions_query(
    cache: &mut ConcreteLineCache<'_>,
    item: &HandsByActionsBenchmarkItem,
) -> Result<Option<LineToHandsByActionsQuery>, ToolError> {
    let concrete_line = cache.get(&item.dimension(), item.concrete_line_id)?;
    if concrete_line.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(LineToHandsByActionsQuery {
        strategy: item.strategy.clone(),
        player_count: item.player_count,
        depth_bb: item.depth_bb,
        concrete_line_id: item.concrete_line_id,
        concrete_line,
        actions: item.actions.clone(),
        frequency: item.frequency,
    }))
}

struct ConcreteLineCache<'a> {
    connection: &'a Connection,
    values: HashMap<(String, u32, u32, u32), String>,
}

impl<'a> ConcreteLineCache<'a> {
    fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            values: HashMap::new(),
        }
    }

    fn get(
        &mut self,
        dimension: &DimensionRef,
        concrete_line_id: u32,
    ) -> Result<String, ToolError> {
        let key = (
            dimension.strategy.clone(),
            dimension.player_count,
            dimension.depth_bb,
            concrete_line_id,
        );
        if let Some(value) = self.values.get(&key) {
            return Ok(value.clone());
        }
        let value = load_concrete_line(self.connection, dimension, concrete_line_id)?;
        self.values.insert(key, value.clone());
        Ok(value)
    }
}

fn load_concrete_line(
    connection: &Connection,
    dimension: &DimensionRef,
    concrete_line_id: u32,
) -> Result<String, ToolError> {
    let table = quote_identifier(&get_concrete_lines_table_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    ))?;
    let sql = format!("SELECT concrete_line FROM {table} WHERE concrete_line_id = ?1");
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[Value::from(concrete_line_id)])?;
    if statement.step_row()? {
        Ok(statement.column_text(0)?)
    } else {
        Err(ToolError::invalid_argument(format!(
            "concrete_line_id={} not found in metadata dimension {}:{}:{}",
            concrete_line_id, dimension.strategy, dimension.player_count, dimension.depth_bb
        )))
    }
}
