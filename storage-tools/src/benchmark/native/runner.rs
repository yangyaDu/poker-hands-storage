use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::benchmark::memory_snapshot::{
    get_memory_snapshot, BenchmarkMemoryReport, MemorySnapshot,
};
use crate::benchmark::metrics::{build_totals, measure_benchmark_case, BenchmarkCaseResult};
use crate::benchmark::report::{
    build_benchmark_report_for_engine, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::types::{
    BenchmarkWorkload, DrillScenarioBenchmarkItem, HandsByActionsBenchmarkItem, WorkloadOptions,
    WorkloadSource,
};
use crate::benchmark::workload::{
    create_benchmark_workload, read_workload_json, write_workload_json,
};
use crate::errors::ToolError;
use range_store_core::dimension::{get_concrete_lines_table_name, quote_identifier, DimensionRef};
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::query::RangeStoreFacade;
use range_store_core::sqlite::{Connection, Value};

use super::types::BenchmarkNativeCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum NativeWorkerMode {
    Core,
    Direct,
    HttpService,
    Sdk,
}

impl NativeWorkerMode {
    fn label(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Direct => "native-direct",
            Self::HttpService => "http-service",
            Self::Sdk => "native-sdk",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeWorkerInput {
    mode: NativeWorkerMode,
    data_dir: String,
    native_entry: String,
    native_node_entry: String,
    #[serde(default)]
    http_base_url: Option<String>,
    max_open_handles: u32,
    verify_checksums: bool,
    warmup_iterations: usize,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConcreteLineLookupQuery {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line_id: u32,
    concrete_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeWorkerOutput {
    cold_start: serde_json::Value,
    cases: Vec<BenchmarkCaseResult>,
    memory_before: MemorySnapshot,
    #[serde(default)]
    memory_after_import: Option<MemorySnapshot>,
    #[serde(default)]
    memory_after_constructor: Option<MemorySnapshot>,
    #[serde(default)]
    memory_after_warmup: Option<MemorySnapshot>,
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
    let mut outputs: HashMap<NativeWorkerMode, NativeWorkerOutput> = HashMap::new();
    let entry_order = benchmark_entry_order(command.seed);
    for mode in &entry_order {
        let output = match mode {
            NativeWorkerMode::Core => run_core_worker_process(
                command,
                workload.clone(),
                concrete_line_queries.clone(),
                line_to_hands_by_actions_queries.clone(),
            )?,
            NativeWorkerMode::HttpService => run_http_service_worker(
                command,
                workload.clone(),
                concrete_line_queries.clone(),
                line_to_hands_by_actions_queries.clone(),
            )?,
            NativeWorkerMode::Direct | NativeWorkerMode::Sdk => run_native_worker(
                command,
                *mode,
                workload.clone(),
                concrete_line_queries.clone(),
                line_to_hands_by_actions_queries.clone(),
            )?,
        };
        outputs.insert(*mode, output);
    }

    let core_output = outputs
        .remove(&NativeWorkerMode::Core)
        .ok_or_else(|| ToolError::invalid_format("missing core benchmark output"))?;
    let direct_output = outputs
        .remove(&NativeWorkerMode::Direct)
        .ok_or_else(|| ToolError::invalid_format("missing native-direct benchmark output"))?;
    let http_output = outputs
        .remove(&NativeWorkerMode::HttpService)
        .ok_or_else(|| ToolError::invalid_format("missing http-service benchmark output"))?;
    let sdk_output = outputs
        .remove(&NativeWorkerMode::Sdk)
        .ok_or_else(|| ToolError::invalid_format("missing native-sdk benchmark output"))?;

    let mut cases = core_output.cases.clone();
    cases.extend(direct_output.cases.clone());
    cases.extend(sdk_output.cases.clone());
    cases.extend(http_output.cases.clone());
    let memory = BenchmarkMemoryReport::new(
        sdk_output.memory_before.clone(),
        sdk_output.memory_after.clone(),
    );
    let totals = build_totals(&cases);
    let mut notes = vec![
        "Fair entry benchmark; storage-tools runs core, native-direct, native-sdk, and http-service in separate child processes using the same workload JSON.".to_owned(),
        format!(
            "Entry execution order is randomized by seed: {}.",
            entry_order
                .iter()
                .map(|mode| mode.label())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "OS page cache is still shared across worker processes; use repeated runs and randomized order before treating small differences as engine differences.".to_owned(),
        "Top-level memory report uses the native-sdk worker only; core and native-direct RSS are recorded in notes to avoid summing stores across processes.".to_owned(),
        "Cold start is measured inside each worker as import/require where applicable + store construction + first hand query + explicit warmup.".to_owned(),
        "Result counts are case-specific: concrete line lookups, abstract lines for drill metadata, action entries, batch action entries, and matching hands.".to_owned(),
        "Exact concrete-line lookup cases skip empty concrete_line rows because the business API treats empty strings as invalid input.".to_owned(),
        "`*:batch-hand-strategy` is the default --batch-size case; `*:batch-size-*` entries are the batch-size sweep and should not be interpreted as separate API semantics.".to_owned(),
    ];
    notes.extend(worker_memory_notes(NativeWorkerMode::Core, &core_output));
    notes.extend(worker_memory_notes(
        NativeWorkerMode::Direct,
        &direct_output,
    ));
    notes.extend(worker_memory_notes(NativeWorkerMode::Sdk, &sdk_output));
    notes.extend(worker_memory_notes(
        NativeWorkerMode::HttpService,
        &http_output,
    ));
    notes.extend(core_output.notes);
    notes.extend(direct_output.notes);
    notes.extend(sdk_output.notes);
    notes.extend(http_output.notes);

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
            cases,
            totals,
            memory,
            result_verification: None,
            notes,
        },
        "bun-native",
    );
    report.cold_start = Some(serde_json::json!({
        "entryOrder": entry_order.iter().map(|mode| mode.label()).collect::<Vec<_>>(),
        "core": core_output.cold_start,
        "direct": direct_output.cold_start,
        "sdk": sdk_output.cold_start,
        "httpService": http_output.cold_start,
    }));

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

fn benchmark_entry_order(seed: u64) -> Vec<NativeWorkerMode> {
    let mut order = vec![
        NativeWorkerMode::Core,
        NativeWorkerMode::Direct,
        NativeWorkerMode::HttpService,
        NativeWorkerMode::Sdk,
    ];
    order.sort_by_key(|mode| entry_order_key(seed, mode.label()));
    order
}

fn entry_order_key(seed: u64, label: &str) -> u64 {
    let mut value = seed ^ 0x9E37_79B9_7F4A_7C15;
    for byte in label.bytes() {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x1000_0000_01B3);
        value ^= value >> 32;
    }
    value
}

fn run_core_worker_process(
    command: &BenchmarkNativeCommand,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
) -> Result<NativeWorkerOutput, ToolError> {
    let input = NativeWorkerInput {
        mode: NativeWorkerMode::Core,
        data_dir: absolute_existing_path(&command.dir, "--dir")?
            .display()
            .to_string(),
        native_entry: absolute_existing_path(&command.native_entry, "--native-entry")?
            .display()
            .to_string(),
        native_node_entry: native_node_entry(&command.native_entry)?
            .display()
            .to_string(),
        http_base_url: None,
        max_open_handles: command.max_open_handles,
        verify_checksums: command.verify_checksums,
        warmup_iterations: command.warmup_iterations,
        workload,
        concrete_line_queries,
        line_to_hands_by_actions_queries,
    };
    let input_path = write_worker_input(&input)?;
    let current_exe = std::env::current_exe()?;
    let output = Command::new(current_exe)
        .arg("benchmark-native-core-worker")
        .arg(&input_path)
        .output()
        .map_err(|error| {
            ToolError::new(
                "CORE_BENCHMARK_FAILED",
                format!("Failed to start core benchmark worker: {error}"),
            )
        })?;
    let _ = fs::remove_file(&input_path);
    parse_worker_output("core benchmark worker", output)
}

fn run_native_worker(
    command: &BenchmarkNativeCommand,
    mode: NativeWorkerMode,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
) -> Result<NativeWorkerOutput, ToolError> {
    let input = NativeWorkerInput {
        mode,
        data_dir: absolute_existing_path(&command.dir, "--dir")?
            .display()
            .to_string(),
        native_entry: absolute_existing_path(&command.native_entry, "--native-entry")?
            .display()
            .to_string(),
        native_node_entry: native_node_entry(&command.native_entry)?
            .display()
            .to_string(),
        http_base_url: None,
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
    parse_worker_output("Bun native benchmark worker", output)
}

fn run_http_service_worker(
    command: &BenchmarkNativeCommand,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineLookupQuery>,
    line_to_hands_by_actions_queries: Vec<LineToHandsByActionsQuery>,
) -> Result<NativeWorkerOutput, ToolError> {
    let service_bin = resolve_http_service_bin(command)?;
    let base_url = start_http_service(command, &service_bin)?;
    let input = NativeWorkerInput {
        mode: NativeWorkerMode::HttpService,
        data_dir: absolute_existing_path(&command.dir, "--dir")?
            .display()
            .to_string(),
        native_entry: absolute_existing_path(&command.native_entry, "--native-entry")?
            .display()
            .to_string(),
        native_node_entry: native_node_entry(&command.native_entry)?
            .display()
            .to_string(),
        http_base_url: Some(base_url.url.clone()),
        max_open_handles: command.max_open_handles,
        verify_checksums: command.verify_checksums,
        warmup_iterations: command.warmup_iterations,
        workload,
        concrete_line_queries,
        line_to_hands_by_actions_queries,
    };
    let input_path = write_worker_input(&input)?;
    let current_exe = std::env::current_exe()?;
    let output = Command::new(current_exe)
        .arg("benchmark-native-http-worker")
        .arg(&input_path)
        .output()
        .map_err(|error| {
            ToolError::new(
                "HTTP_BENCHMARK_FAILED",
                format!("Failed to start HTTP benchmark worker: {error}"),
            )
        });
    let _ = fs::remove_file(&input_path);
    stop_http_service(base_url.child);
    parse_worker_output("HTTP service benchmark worker", output?)
}

fn parse_worker_output(
    label: &str,
    output: std::process::Output,
) -> Result<NativeWorkerOutput, ToolError> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(ToolError::new(
            "ENTRY_BENCHMARK_FAILED",
            format!(
                "{label} failed with status {:?}. stderr: {} stdout: {}",
                output.status.code(),
                stderr.trim(),
                stdout.trim()
            ),
        ));
    }
    serde_json::from_str::<NativeWorkerOutput>(&stdout).map_err(|error| {
        ToolError::invalid_format(format!(
            "{label} returned invalid JSON: {error}. stdout: {} stderr: {}",
            stdout.trim(),
            stderr.trim()
        ))
    })
}

pub fn run_core_worker_from_input_path(input_path: &Path) -> Result<String, ToolError> {
    let input_json = fs::read_to_string(input_path)?;
    let input: NativeWorkerInput = serde_json::from_str(&input_json)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let output = run_core_worker(input)?;
    serde_json::to_string(&output).map_err(|error| ToolError::invalid_format(error.to_string()))
}

pub fn run_http_worker_from_input_path(input_path: &Path) -> Result<String, ToolError> {
    let input_json = fs::read_to_string(input_path)?;
    let input: NativeWorkerInput = serde_json::from_str(&input_json)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let output = run_http_worker(input)?;
    serde_json::to_string(&output).map_err(|error| ToolError::invalid_format(error.to_string()))
}

fn run_core_worker(input: NativeWorkerInput) -> Result<NativeWorkerOutput, ToolError> {
    let worker_start = Instant::now();
    let memory_before = get_memory_snapshot();
    let constructor_start = Instant::now();
    let store = RangeStoreFacade::open(
        PathBuf::from(&input.data_dir),
        input.max_open_handles as usize,
        input.verify_checksums,
    )
    .map_err(|error| ToolError::new("CORE_BENCHMARK_FAILED", error.to_string()))?;
    let constructor_ms = elapsed_ms(constructor_start);
    let memory_after_constructor = get_memory_snapshot();

    let mut first_query_ms = 0.0;
    let mut first_query_result_count = 0usize;
    let mut first_query = serde_json::Value::Null;
    let mut stats_after_first_query = serde_json::Value::Null;
    if let Some(item) = input.workload.hand_queries.first() {
        first_query = serde_json::to_value(item)
            .map_err(|error| ToolError::invalid_format(error.to_string()))?;
        let first_query_start = Instant::now();
        let result = store
            .query_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)
            .map_err(|error| ToolError::new("CORE_BENCHMARK_FAILED", error.to_string()))?;
        first_query_ms = elapsed_ms(first_query_start);
        first_query_result_count = result.actions.len();
        stats_after_first_query = store_stats_json(&store);
    }
    let memory_after_first_query = get_memory_snapshot();

    let warmup_start = Instant::now();
    warmup_core_store(&store, &input);
    let warmup_ms = elapsed_ms(warmup_start);
    let memory_after_warmup = get_memory_snapshot();

    let cases = measure_core_cases(&store, &input);
    let memory_after = get_memory_snapshot();
    let stats_after_benchmark = store_stats_json(&store);
    let cold_start = serde_json::json!({
        "mode": "core-worker",
        "constructorMs": constructor_ms,
        "firstQueryMs": first_query_ms,
        "warmupMs": warmup_ms,
        "totalMs": elapsed_ms(worker_start),
        "firstQueryResultCount": first_query_result_count,
        "firstQuery": first_query,
        "statsAfterFirstQuery": stats_after_first_query,
        "statsAfterBenchmark": stats_after_benchmark,
        "memoryAfterConstructor": memory_after_constructor,
        "memoryAfterFirstQuery": memory_after_first_query,
        "memoryAfterWarmup": memory_after_warmup,
    });

    Ok(NativeWorkerOutput {
        cold_start,
        cases,
        memory_before,
        memory_after_import: None,
        memory_after_constructor: Some(memory_after_constructor),
        memory_after_warmup: Some(memory_after_warmup),
        memory_after,
        notes: vec![format!(
            "Core worker stats after benchmark: schemaCount={}, openHandleCount={}",
            store.schema_count(),
            store.open_handle_count()
        )],
    })
}

fn run_http_worker(input: NativeWorkerInput) -> Result<NativeWorkerOutput, ToolError> {
    let worker_start = Instant::now();
    let memory_before = get_memory_snapshot();
    let base_url = input
        .http_base_url
        .clone()
        .ok_or_else(|| ToolError::invalid_argument("http_base_url is required"))?;
    let memory_after_constructor = get_memory_snapshot();

    let mut first_query_ms = 0.0;
    let mut first_query_result_count = 0usize;
    let mut first_query = serde_json::Value::Null;
    let mut stats_after_first_query = serde_json::Value::Null;
    if let Some(item) = input.workload.hand_queries.first() {
        first_query = serde_json::to_value(item)
            .map_err(|error| ToolError::invalid_format(error.to_string()))?;
        let first_query_start = Instant::now();
        first_query_result_count = http_hand_strategy_count(&base_url, item)
            .map_err(|error| ToolError::new("HTTP_BENCHMARK_FAILED", error))?;
        first_query_ms = elapsed_ms(first_query_start);
        stats_after_first_query = http_ready_json(&base_url)
            .map_err(|error| ToolError::new("HTTP_BENCHMARK_FAILED", error))?;
    }
    let memory_after_first_query = get_memory_snapshot();

    let warmup_start = Instant::now();
    warmup_http_service(&base_url, &input);
    let warmup_ms = elapsed_ms(warmup_start);
    let memory_after_warmup = get_memory_snapshot();

    let cases = measure_http_cases(&base_url, &input);
    let memory_after = get_memory_snapshot();
    let stats_after_benchmark =
        http_ready_json(&base_url).unwrap_or_else(|error| serde_json::json!({ "error": error }));
    let cold_start = serde_json::json!({
        "mode": "http-service-client-worker",
        "baseUrl": base_url,
        "constructorMs": 0.0,
        "firstQueryMs": first_query_ms,
        "warmupMs": warmup_ms,
        "totalMs": elapsed_ms(worker_start),
        "firstQueryResultCount": first_query_result_count,
        "firstQuery": first_query,
        "statsAfterFirstQuery": stats_after_first_query,
        "statsAfterBenchmark": stats_after_benchmark,
        "memoryAfterConstructor": memory_after_constructor,
        "memoryAfterFirstQuery": memory_after_first_query,
        "memoryAfterWarmup": memory_after_warmup,
    });

    Ok(NativeWorkerOutput {
        cold_start,
        cases,
        memory_before,
        memory_after_import: None,
        memory_after_constructor: Some(memory_after_constructor),
        memory_after_warmup: Some(memory_after_warmup),
        memory_after,
        notes: vec![
            "HTTP service worker measures loopback HTTP request latency against a separately started poker-hands-storage-service process.".to_owned(),
            "HTTP service process RSS is not included in this client worker memory report; use service-level monitoring for production RSS.".to_owned(),
        ],
    })
}

fn measure_core_cases(
    store: &RangeStoreFacade,
    input: &NativeWorkerInput,
) -> Vec<BenchmarkCaseResult> {
    let prefix = "core";
    let mut cases = Vec::new();
    cases.push(measure_benchmark_case(
        "core:concrete-lines-exact",
        "Resolve concrete_line through core RangeStoreFacade get_concrete_lines exact lookup.",
        &input.concrete_line_queries,
        input.warmup_iterations,
        |item, _| core_resolve_concrete_line_id(store, item).map(|_| 1),
    ));
    cases.push(measure_benchmark_case(
        "core:hand-strategy",
        "Single concrete_line_id + hand query through core RangeStoreFacade.",
        &input.workload.hand_queries,
        input.warmup_iterations,
        |item, _| {
            store
                .query_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)
                .map(|result| result.actions.len())
                .map_err(|error| error.to_string())
        },
    ));
    cases.push(measure_benchmark_case(
        "core:batch-hand-strategy",
        "Run the default batch-size concrete_line_id + hand lookup case through core RangeStoreFacade.",
        &input.workload.batch_queries,
        input.warmup_iterations,
        |item, _| core_batch_action_count(store, item),
    ));
    for (size, queries) in &input.workload.batch_queries_by_size {
        cases.push(measure_benchmark_case(
            &format!("{prefix}:batch-size-{size}"),
            &format!("Run {size} lookups per batch through core RangeStoreFacade."),
            queries,
            input.warmup_iterations,
            |item, _| core_batch_action_count(store, item),
        ));
    }
    cases.push(measure_benchmark_case(
        "core:hands-by-actions",
        "Decode all hands for one concrete line through core RangeStoreFacade.",
        &input.workload.hands_by_actions_queries,
        input.warmup_iterations,
        |item, _| core_hands_by_actions_count(store, item, item.concrete_line_id),
    ));
    cases.push(measure_benchmark_case(
        "core:drill-scenarios-metadata",
        "Read drill scenario abstract lines through core RangeStoreFacade cached metadata.",
        &input.workload.drill_scenario_queries,
        input.warmup_iterations,
        |item, _| core_drill_scenario_line_count(store, item),
    ));
    cases.push(measure_benchmark_case(
        "core:line-to-hands-by-actions",
        "Resolve concrete_line and then run handsByActions through core RangeStoreFacade.",
        &input.line_to_hands_by_actions_queries,
        input.warmup_iterations,
        |item, _| {
            let concrete_line_id = core_resolve_line_to_hands_concrete_line_id(store, item)?;
            core_line_to_hands_by_actions_count(store, item, concrete_line_id)
        },
    ));
    cases
}

fn warmup_core_store(store: &RangeStoreFacade, input: &NativeWorkerInput) {
    let warmup = input.warmup_iterations;
    for item in input.concrete_line_queries.iter().take(warmup) {
        let _ = core_resolve_concrete_line_id(store, item);
    }
    for item in input.workload.hand_queries.iter().take(warmup) {
        let _ =
            store.query_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards);
    }
    for item in input.workload.batch_queries.iter().take(warmup) {
        let _ = core_batch_action_count(store, item);
    }
    for (_, queries) in &input.workload.batch_queries_by_size {
        for item in queries.iter().take(warmup) {
            let _ = core_batch_action_count(store, item);
        }
    }
    for item in input.workload.hands_by_actions_queries.iter().take(warmup) {
        let _ = core_hands_by_actions_count(store, item, item.concrete_line_id);
    }
    for item in input.workload.drill_scenario_queries.iter().take(warmup) {
        let _ = core_drill_scenario_line_count(store, item);
    }
    for item in input.line_to_hands_by_actions_queries.iter().take(warmup) {
        if let Ok(concrete_line_id) = core_resolve_line_to_hands_concrete_line_id(store, item) {
            let _ = core_line_to_hands_by_actions_count(store, item, concrete_line_id);
        }
    }
}

fn measure_http_cases(base_url: &str, input: &NativeWorkerInput) -> Vec<BenchmarkCaseResult> {
    let prefix = "http-service";
    let mut cases = Vec::new();
    cases.push(measure_benchmark_case(
        "http-service:concrete-lines-exact",
        "Resolve concrete_line through HTTP /range/concrete-lines exact lookup.",
        &input.concrete_line_queries,
        input.warmup_iterations,
        |item, _| http_resolve_concrete_line_id(base_url, item).map(|_| 1),
    ));
    cases.push(measure_benchmark_case(
        "http-service:hand-strategy",
        "Single concrete_line_id + hand query through HTTP /range/hand-strategy.",
        &input.workload.hand_queries,
        input.warmup_iterations,
        |item, _| http_hand_strategy_count(base_url, item),
    ));
    cases.push(measure_benchmark_case(
        "http-service:batch-hand-strategy",
        "Run the default batch-size concrete_line_id + hand lookup case through HTTP /range/hand-strategy-batch.",
        &input.workload.batch_queries,
        input.warmup_iterations,
        |item, _| http_batch_action_count(base_url, item),
    ));
    for (size, queries) in &input.workload.batch_queries_by_size {
        cases.push(measure_benchmark_case(
            &format!("{prefix}:batch-size-{size}"),
            &format!("Run {size} lookups per batch through HTTP /range/hand-strategy-batch."),
            queries,
            input.warmup_iterations,
            |item, _| http_batch_action_count(base_url, item),
        ));
    }
    cases.push(measure_benchmark_case(
        "http-service:hands-by-actions",
        "Decode all hands for one concrete line through HTTP /range/hands-by-actions.",
        &input.workload.hands_by_actions_queries,
        input.warmup_iterations,
        |item, _| http_hands_by_actions_count(base_url, item, item.concrete_line_id),
    ));
    cases.push(measure_benchmark_case(
        "http-service:drill-scenarios-metadata",
        "Read drill scenario abstract lines through HTTP /range/drill-scenarios.",
        &input.workload.drill_scenario_queries,
        input.warmup_iterations,
        |item, _| http_drill_scenario_line_count(base_url, item),
    ));
    cases.push(measure_benchmark_case(
        "http-service:line-to-hands-by-actions",
        "Resolve concrete_line and then run handsByActions through HTTP endpoints.",
        &input.line_to_hands_by_actions_queries,
        input.warmup_iterations,
        |item, _| {
            let concrete_line_id = http_resolve_line_to_hands_concrete_line_id(base_url, item)?;
            http_line_to_hands_by_actions_count(base_url, item, concrete_line_id)
        },
    ));
    cases
}

fn warmup_http_service(base_url: &str, input: &NativeWorkerInput) {
    let warmup = input.warmup_iterations;
    for item in input.concrete_line_queries.iter().take(warmup) {
        let _ = http_resolve_concrete_line_id(base_url, item);
    }
    for item in input.workload.hand_queries.iter().take(warmup) {
        let _ = http_hand_strategy_count(base_url, item);
    }
    for item in input.workload.batch_queries.iter().take(warmup) {
        let _ = http_batch_action_count(base_url, item);
    }
    for (_, queries) in &input.workload.batch_queries_by_size {
        for item in queries.iter().take(warmup) {
            let _ = http_batch_action_count(base_url, item);
        }
    }
    for item in input.workload.hands_by_actions_queries.iter().take(warmup) {
        let _ = http_hands_by_actions_count(base_url, item, item.concrete_line_id);
    }
    for item in input.workload.drill_scenario_queries.iter().take(warmup) {
        let _ = http_drill_scenario_line_count(base_url, item);
    }
    for item in input.line_to_hands_by_actions_queries.iter().take(warmup) {
        if let Ok(concrete_line_id) = http_resolve_line_to_hands_concrete_line_id(base_url, item) {
            let _ = http_line_to_hands_by_actions_count(base_url, item, concrete_line_id);
        }
    }
}

fn core_resolve_concrete_line_id(
    store: &RangeStoreFacade,
    item: &ConcreteLineLookupQuery,
) -> Result<u32, String> {
    let dimension = DimensionRef::new(item.strategy.clone(), item.player_count, item.depth_bb);
    let lines = store
        .get_concrete_lines(
            &dimension,
            ConcreteLineFilter::Concrete(&item.concrete_line),
        )
        .map_err(|error| error.to_string())?;
    if lines.len() != 1 {
        return Err(format!("expected one concrete line, got {}", lines.len()));
    }
    let concrete_line_id = lines[0].concrete_line_id;
    if concrete_line_id != item.concrete_line_id {
        return Err(format!(
            "concrete line id mismatch: expected {}, got {}",
            item.concrete_line_id, concrete_line_id
        ));
    }
    Ok(concrete_line_id)
}

fn core_batch_action_count(
    store: &RangeStoreFacade,
    item: &crate::benchmark::types::BatchBenchmarkItem,
) -> Result<usize, String> {
    let requests = item
        .requests
        .iter()
        .map(|request| (request.concrete_line_id, request.hole_cards.clone()))
        .collect::<Vec<_>>();
    let results = store
        .query_batch(&item.dimension(), &requests)
        .map_err(|error| error.to_string())?;
    let mut total = 0usize;
    for result in results {
        if let Some(error) = result.error {
            return Err(error);
        }
        if let Some(actions) = result.actions {
            total += actions.len();
        }
    }
    Ok(total)
}

fn core_resolve_line_to_hands_concrete_line_id(
    store: &RangeStoreFacade,
    item: &LineToHandsByActionsQuery,
) -> Result<u32, String> {
    let dimension = DimensionRef::new(item.strategy.clone(), item.player_count, item.depth_bb);
    let lines = store
        .get_concrete_lines(
            &dimension,
            ConcreteLineFilter::Concrete(&item.concrete_line),
        )
        .map_err(|error| error.to_string())?;
    if lines.len() != 1 {
        return Err(format!("expected one concrete line, got {}", lines.len()));
    }
    let concrete_line_id = lines[0].concrete_line_id;
    if concrete_line_id != item.concrete_line_id {
        return Err(format!(
            "concrete line id mismatch: expected {}, got {}",
            item.concrete_line_id, concrete_line_id
        ));
    }
    Ok(concrete_line_id)
}

fn core_hands_by_actions_count(
    store: &RangeStoreFacade,
    item: &HandsByActionsBenchmarkItem,
    concrete_line_id: u32,
) -> Result<usize, String> {
    store
        .hands_by_action_names(
            &item.dimension(),
            concrete_line_id,
            &item.actions,
            item.frequency,
        )
        .map(|hands| hands.len())
        .map_err(|error| error.to_string())
}

fn core_drill_scenario_line_count(
    store: &RangeStoreFacade,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, String> {
    store
        .get_drill_scenario_lines(
            &item.strategy,
            &item.drill_name,
            item.player_count,
            item.drill_depth,
        )
        .map(|lines| lines.len())
        .map_err(|error| error.to_string())
}

fn core_line_to_hands_by_actions_count(
    store: &RangeStoreFacade,
    item: &LineToHandsByActionsQuery,
    concrete_line_id: u32,
) -> Result<usize, String> {
    let dimension = DimensionRef::new(item.strategy.clone(), item.player_count, item.depth_bb);
    store
        .hands_by_action_names(&dimension, concrete_line_id, &item.actions, item.frequency)
        .map(|hands| hands.len())
        .map_err(|error| error.to_string())
}

fn http_resolve_concrete_line_id(
    base_url: &str,
    item: &ConcreteLineLookupQuery,
) -> Result<u32, String> {
    let value = http_post_json(
        base_url,
        "/range/concrete-lines",
        serde_json::json!({
            "strategy": item.strategy,
            "player_count": item.player_count,
            "depth_bb": item.depth_bb,
            "concrete_line": item.concrete_line,
        }),
    )?;
    let data = api_data(&value)?;
    let lines = data
        .get("lines")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "HTTP concrete-lines response missing data.lines".to_owned())?;
    if lines.len() != 1 {
        return Err(format!("expected one concrete line, got {}", lines.len()));
    }
    let concrete_line_id = lines[0]
        .get("concrete_line_id")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| "HTTP concrete-lines response missing concrete_line_id".to_owned())?
        as u32;
    if concrete_line_id != item.concrete_line_id {
        return Err(format!(
            "concrete line id mismatch: expected {}, got {}",
            item.concrete_line_id, concrete_line_id
        ));
    }
    Ok(concrete_line_id)
}

fn http_hand_strategy_count(
    base_url: &str,
    item: &crate::benchmark::types::HandBenchmarkItem,
) -> Result<usize, String> {
    let value = http_post_json(
        base_url,
        "/range/hand-strategy",
        serde_json::json!({
            "strategy": item.strategy,
            "player_count": item.player_count,
            "depth_bb": item.depth_bb,
            "concrete_line_id": item.concrete_line_id,
            "hole_cards": item.hole_cards,
        }),
    )?;
    api_data(&value)?
        .get("actions")
        .and_then(|value| value.as_array())
        .map(|actions| actions.len())
        .ok_or_else(|| "HTTP hand-strategy response missing data.actions".to_owned())
}

fn http_batch_action_count(
    base_url: &str,
    item: &crate::benchmark::types::BatchBenchmarkItem,
) -> Result<usize, String> {
    let requests = item
        .requests
        .iter()
        .map(|request| {
            serde_json::json!({
                "concrete_line_id": request.concrete_line_id,
                "hole_cards": request.hole_cards,
            })
        })
        .collect::<Vec<_>>();
    let value = http_post_json(
        base_url,
        "/range/hand-strategy-batch",
        serde_json::json!({
            "strategy": item.strategy,
            "player_count": item.player_count,
            "depth_bb": item.depth_bb,
            "requests": requests,
        }),
    )?;
    let results = api_data(&value)?
        .get("results")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "HTTP batch response missing data.results".to_owned())?;
    let mut total = 0usize;
    for result in results {
        if let Some(error) = result.get("error").filter(|value| !value.is_null()) {
            return Err(error
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("HTTP batch item failed")
                .to_owned());
        }
        let actions = result
            .get("strategy")
            .and_then(|strategy| strategy.get("actions"))
            .and_then(|value| value.as_array())
            .ok_or_else(|| "HTTP batch item missing strategy.actions".to_owned())?;
        total += actions.len();
    }
    Ok(total)
}

fn http_resolve_line_to_hands_concrete_line_id(
    base_url: &str,
    item: &LineToHandsByActionsQuery,
) -> Result<u32, String> {
    let query = ConcreteLineLookupQuery {
        strategy: item.strategy.clone(),
        player_count: item.player_count,
        depth_bb: item.depth_bb,
        concrete_line_id: item.concrete_line_id,
        concrete_line: item.concrete_line.clone(),
    };
    http_resolve_concrete_line_id(base_url, &query)
}

fn http_hands_by_actions_count(
    base_url: &str,
    item: &HandsByActionsBenchmarkItem,
    concrete_line_id: u32,
) -> Result<usize, String> {
    let mut body = serde_json::json!({
        "strategy": item.strategy,
        "player_count": item.player_count,
        "depth_bb": item.depth_bb,
        "concrete_line_id": concrete_line_id,
        "actions": item.actions,
    });
    if let Some(frequency) = item.frequency {
        body["frequency"] = serde_json::json!(frequency);
    }
    let value = http_post_json(base_url, "/range/hands-by-actions", body)?;
    api_data(&value)?
        .get("hands")
        .and_then(|value| value.as_array())
        .map(|hands| hands.len())
        .ok_or_else(|| "HTTP hands-by-actions response missing data.hands".to_owned())
}

fn http_line_to_hands_by_actions_count(
    base_url: &str,
    item: &LineToHandsByActionsQuery,
    concrete_line_id: u32,
) -> Result<usize, String> {
    let query = HandsByActionsBenchmarkItem {
        strategy: item.strategy.clone(),
        player_count: item.player_count,
        depth_bb: item.depth_bb,
        concrete_line_id,
        actions: item.actions.clone(),
        frequency: item.frequency,
    };
    http_hands_by_actions_count(base_url, &query, concrete_line_id)
}

fn http_drill_scenario_line_count(
    base_url: &str,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, String> {
    let value = http_post_json(
        base_url,
        "/range/drill-scenarios",
        serde_json::json!({
            "strategy": item.strategy,
            "drill_name": item.drill_name,
            "player_count": item.player_count,
            "drill_depth": item.drill_depth,
        }),
    )?;
    api_data(&value)?
        .get("abstract_lines")
        .and_then(|value| value.as_array())
        .map(|lines| lines.len())
        .ok_or_else(|| "HTTP drill-scenarios response missing data.abstract_lines".to_owned())
}

fn store_stats_json(store: &RangeStoreFacade) -> serde_json::Value {
    serde_json::json!({
        "schemaCount": store.schema_count(),
        "openHandleCount": store.open_handle_count(),
        "knownDimensions": store.known_dimensions(),
    })
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn worker_memory_notes(mode: NativeWorkerMode, output: &NativeWorkerOutput) -> Vec<String> {
    let delta = match (
        output.memory_before.rss_bytes,
        output.memory_after.rss_bytes,
    ) {
        (Some(before), Some(after)) => Some(after as i64 - before as i64),
        _ => None,
    };
    let mut notes = vec![format!(
        "{} worker RSS total: baseline={}, afterBenchmark={}, delta={}",
        mode.label(),
        format_optional_bytes(output.memory_before.rss_bytes),
        format_optional_bytes(output.memory_after.rss_bytes),
        format_optional_i64(delta)
    )];
    notes.push(format!(
        "{} worker RSS phases: baseline={}, afterImport={}, afterConstructor={}, afterWarmup={}, afterBenchmark={}",
        mode.label(),
        format_optional_bytes(output.memory_before.rss_bytes),
        format_optional_bytes(output.memory_after_import.as_ref().and_then(|snapshot| snapshot.rss_bytes)),
        format_optional_bytes(output.memory_after_constructor.as_ref().and_then(|snapshot| snapshot.rss_bytes)),
        format_optional_bytes(output.memory_after_warmup.as_ref().and_then(|snapshot| snapshot.rss_bytes)),
        format_optional_bytes(output.memory_after.rss_bytes),
    ));
    notes
}

fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn format_optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unavailable".to_owned())
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

struct HttpServiceProcess {
    url: String,
    child: Child,
}

fn resolve_http_service_bin(command: &BenchmarkNativeCommand) -> Result<PathBuf, ToolError> {
    if let Some(path) = &command.http_service_bin {
        return absolute_existing_path(path, "--http-service-bin");
    }
    let current_exe = std::env::current_exe()?;
    let sibling = current_exe
        .parent()
        .ok_or_else(|| ToolError::invalid_argument("current executable has no parent directory"))?
        .join(format!(
            "poker-hands-storage-service{}",
            std::env::consts::EXE_SUFFIX
        ));
    absolute_existing_path(&sibling, "--http-service-bin").map_err(|_| {
        ToolError::invalid_argument(format!(
            "HTTP service binary was not found next to storage-tools: {}. Build it first or pass --http-service-bin.",
            sibling.display()
        ))
    })
}

fn start_http_service(
    command: &BenchmarkNativeCommand,
    service_bin: &Path,
) -> Result<HttpServiceProcess, ToolError> {
    let port = allocate_loopback_port()?;
    let bind = format!("127.0.0.1:{port}");
    let url = format!("http://{bind}");
    let data_dir = absolute_existing_path(&command.dir, "--dir")?;
    let meta_db = absolute_existing_path(&command.meta, "--meta")?;
    let mut child = Command::new(service_bin)
        .arg("serve")
        .env("PHS_BIND", &bind)
        .env("PHS_DATA_DIR", data_dir)
        .env("PHS_META_DB", meta_db)
        .env("PHS_MAX_OPEN_HANDLES", command.max_open_handles.to_string())
        .env(
            "PHS_VERIFY_CHECKSUMS",
            if command.verify_checksums {
                "true"
            } else {
                "false"
            },
        )
        .env("PHS_PREWARM", "")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            ToolError::new(
                "HTTP_SERVICE_FAILED",
                format!(
                    "Failed to start HTTP service `{}`: {error}",
                    service_bin.display()
                ),
            )
        })?;
    if let Err(error) = wait_for_http_ready(&url, &mut child) {
        stop_http_service(child);
        return Err(error);
    }
    Ok(HttpServiceProcess { url, child })
}

fn stop_http_service(mut child: Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn allocate_loopback_port() -> Result<u16, ToolError> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn wait_for_http_ready(url: &str, child: &mut Child) -> Result<(), ToolError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            return Err(ToolError::new(
                "HTTP_SERVICE_FAILED",
                format!("HTTP service exited before readiness: {status}"),
            ));
        }
        if http_ready_json(url).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(ToolError::new(
        "HTTP_SERVICE_FAILED",
        format!("HTTP service did not become ready: {url}/ready"),
    ))
}

#[derive(Debug, Clone)]
struct HttpEndpoint {
    host: String,
    port: u16,
}

fn http_ready_json(base_url: &str) -> Result<serde_json::Value, String> {
    http_get_json(base_url, "/ready").and_then(|value| {
        if value.get("code").and_then(|code| code.as_i64()) == Some(0) {
            Ok(value)
        } else {
            Err(format!("HTTP ready returned non-zero code: {value}"))
        }
    })
}

fn http_get_json(base_url: &str, path: &str) -> Result<serde_json::Value, String> {
    http_request_json(base_url, "GET", path, None)
}

fn http_post_json(
    base_url: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    http_request_json(base_url, "POST", path, Some(body))
}

fn http_request_json(
    base_url: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let endpoint = parse_http_endpoint(base_url)?;
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .map_err(|error| format!("connect {base_url} failed: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| error.to_string())?;

    let body_bytes = body
        .map(|value| serde_json::to_vec(&value).map_err(|error| error.to_string()))
        .transpose()?
        .unwrap_or_default();
    let content_headers = if body_bytes.is_empty() {
        String::new()
    } else {
        format!(
            "content-type: application/json\r\ncontent-length: {}\r\n",
            body_bytes.len()
        )
    };
    let request = format!(
        "{method} {path} HTTP/1.1\r\nhost: {}:{}\r\naccept: application/json\r\nconnection: close\r\n{content_headers}\r\n",
        endpoint.host, endpoint.port
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&body_bytes))
        .map_err(|error| format!("write HTTP request failed: {error}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| format!("read HTTP response failed: {error}"))?;
    parse_http_response(&response)
}

fn parse_http_endpoint(base_url: &str) -> Result<HttpEndpoint, String> {
    let rest = base_url
        .strip_prefix("http://")
        .ok_or_else(|| format!("Only http:// URLs are supported: {base_url}"))?;
    let host_port = rest.trim_end_matches('/');
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or_else(|| format!("HTTP URL must include a port: {base_url}"))?;
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("HTTP URL has invalid port: {base_url}"))?;
    Ok(HttpEndpoint {
        host: host.to_owned(),
        port,
    })
}

fn parse_http_response(response: &[u8]) -> Result<serde_json::Value, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response missing header terminator".to_owned())?;
    let header_bytes = &response[..header_end];
    let body_bytes = &response[header_end + 4..];
    let headers = String::from_utf8_lossy(header_bytes);
    let mut lines = headers.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "HTTP response missing status line".to_owned())?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("HTTP response has invalid status line: {status_line}"))?;
    let is_chunked = headers
        .lines()
        .any(|line| line.to_ascii_lowercase().trim() == "transfer-encoding: chunked");
    let body = if is_chunked {
        decode_chunked_body(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    let value = serde_json::from_slice::<serde_json::Value>(&body)
        .map_err(|error| format!("HTTP response body is not JSON: {error}"))?;
    if !(200..300).contains(&status_code) {
        return Err(format!("HTTP status {status_code}: {value}"));
    }
    Ok(value)
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut index = 0usize;
    let mut decoded = Vec::new();
    loop {
        let line_end = body[index..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| index + offset)
            .ok_or_else(|| "chunked response missing chunk size".to_owned())?;
        let size_text = std::str::from_utf8(&body[index..line_end])
            .map_err(|error| format!("invalid chunk size utf8: {error}"))?;
        let size_hex = size_text.split(';').next().unwrap_or(size_text).trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("invalid chunk size: {size_text}"))?;
        index = line_end + 2;
        if size == 0 {
            break;
        }
        let chunk_end = index + size;
        if chunk_end + 2 > body.len() {
            return Err("chunked response ended early".to_owned());
        }
        decoded.extend_from_slice(&body[index..chunk_end]);
        index = chunk_end + 2;
    }
    Ok(decoded)
}

fn api_data(value: &serde_json::Value) -> Result<&serde_json::Value, String> {
    if value.get("code").and_then(|code| code.as_i64()) != Some(0) {
        return Err(format!("HTTP API returned non-zero code: {value}"));
    }
    value
        .get("data")
        .filter(|data| !data.is_null())
        .ok_or_else(|| format!("HTTP API response missing data: {value}"))
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
