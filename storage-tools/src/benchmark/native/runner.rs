use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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
    Sdk,
}

impl NativeWorkerMode {
    fn label(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::Direct => "native-direct",
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
    let sdk_output = outputs
        .remove(&NativeWorkerMode::Sdk)
        .ok_or_else(|| ToolError::invalid_format("missing native-sdk benchmark output"))?;

    let mut cases = core_output.cases.clone();
    cases.extend(direct_output.cases.clone());
    cases.extend(sdk_output.cases.clone());
    let memory = BenchmarkMemoryReport::new(
        sdk_output.memory_before.clone(),
        sdk_output.memory_after.clone(),
    );
    let totals = build_totals(&cases);
    let mut notes = vec![
        "Fair entry benchmark; storage-tools runs core, native-direct, and native-sdk in separate child processes using the same workload JSON.".to_owned(),
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
    notes.extend(core_output.notes);
    notes.extend(direct_output.notes);
    notes.extend(sdk_output.notes);

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
