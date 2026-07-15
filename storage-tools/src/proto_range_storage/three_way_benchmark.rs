use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use range_store_core::dimension::{quote_identifier, DimensionRef};
use range_store_core::metadata::ConcreteLineFilter;
use range_store_core::query::{
    parse_action_filters, ActionFilter, FrequencyFilter, RangeStoreFacade,
    DEFAULT_HANDS_BY_ACTIONS_FREQUENCY,
};
use range_store_core::sqlite::{Connection, Value};
use serde::{Deserialize, Serialize};

use crate::benchmark::memory_snapshot::{
    get_memory_snapshot, BenchmarkMemoryReport, MemorySnapshot,
};
use crate::benchmark::metrics::{
    build_totals, measure_benchmark_case, safe_ratio, BenchmarkCaseResult, BenchmarkTotals,
};
use crate::benchmark::report::{BenchmarkOptionsSummary, BenchmarkWorkloadSummary};
use crate::benchmark::report_support::{
    format_binary_bytes, generated_at_utc, markdown_table, write_json_report, write_markdown_report,
};
use crate::benchmark::types::{
    concrete_lines_table_name, drill_scenario_table_name, range_table_name, BatchBenchmarkItem,
    BenchmarkWorkload, ConcreteLineBenchmarkItem, DrillScenarioBenchmarkItem,
    HandsByActionsBenchmarkItem, WorkloadMode, WorkloadOptions, WorkloadSource,
};
use crate::benchmark::workload::{
    build_concrete_line_lookup_queries, create_benchmark_workload, drill_depth_column,
    read_workload_json, write_workload_json, ConcreteLineIdColumn,
};
use crate::errors::ToolError;

use super::query_facade::ProtoRangeStoreFacade;

const ACTION_AMOUNT_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeWayHotBenchmarkCommand {
    pub source_db: PathBuf,
    pub proto_root: PathBuf,
    pub core_dir: PathBuf,
    pub core_meta: PathBuf,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
    pub workload_path: Option<PathBuf>,
    pub write_workload_path: Option<PathBuf>,
    pub seed: u64,
    pub hand_iterations: usize,
    pub batch_iterations: usize,
    pub batch_size: usize,
    pub batch_sizes: Vec<usize>,
    pub requested_dimensions: Vec<DimensionRef>,
    pub requested_dimension_values: Vec<String>,
    pub workload_mode: WorkloadMode,
    pub warmup_iterations: usize,
    pub max_open_handles: usize,
    pub verify_checksums: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayHotBenchmarkReport {
    pub generated_at: String,
    pub semantic_profile: String,
    pub source_db_path: String,
    pub proto_storage_root: String,
    pub core_dir: String,
    pub core_meta_path: String,
    pub options: BenchmarkOptionsSummary,
    pub workload: BenchmarkWorkloadSummary,
    pub workload_source: String,
    pub workload_path: Option<String>,
    pub cases: Vec<ThreeWayBenchmarkCase>,
    pub excluded_cases: Vec<ExcludedBenchmarkCase>,
    pub totals: ThreeWayBenchmarkTotals,
    pub memory: ThreeWayMemoryReport,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayBenchmarkCase {
    pub name: String,
    pub core: BenchmarkCaseResult,
    pub proto: BenchmarkCaseResult,
    pub sqlite: BenchmarkCaseResult,
    pub core_to_sqlite_avg_latency_ratio: f64,
    pub proto_to_sqlite_avg_latency_ratio: f64,
    pub proto_to_core_avg_latency_ratio: f64,
    pub core_to_sqlite_p95_latency_ratio: f64,
    pub proto_to_sqlite_p95_latency_ratio: f64,
    pub proto_to_core_p95_latency_ratio: f64,
    pub result_count_match: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedBenchmarkCase {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayBenchmarkTotals {
    pub core: BenchmarkTotals,
    pub proto: BenchmarkTotals,
    pub sqlite: BenchmarkTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayMemoryReport {
    pub core: ThreeWayEngineMemoryReport,
    pub proto: ThreeWayEngineMemoryReport,
    pub sqlite: ThreeWayEngineMemoryReport,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayEngineMemoryReport {
    pub open: BenchmarkMemoryReport,
    pub prewarm_increment: BenchmarkMemoryReport,
    pub total: BenchmarkMemoryReport,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThreeWayMemoryEngine {
    Core,
    Proto,
    Sqlite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreeWayMemoryWorkerInput {
    engine: ThreeWayMemoryEngine,
    source_db: PathBuf,
    proto_root: PathBuf,
    core_dir: PathBuf,
    core_meta: PathBuf,
    workload: BenchmarkWorkload,
    concrete_line_queries: Vec<ConcreteLineBenchmarkItem>,
    max_open_handles: usize,
    verify_checksums: bool,
    warmup_iterations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreeWayMemoryWorkerOutput {
    before: MemorySnapshot,
    after_open: MemorySnapshot,
    after_prewarm: MemorySnapshot,
    notes: Vec<String>,
}

impl ThreeWayHotBenchmarkReport {
    pub fn has_errors(&self) -> bool {
        self.totals.core.error_count > 0
            || self.totals.proto.error_count > 0
            || self.totals.sqlite.error_count > 0
    }
}

pub fn run_three_way_hot_benchmark(
    command: &ThreeWayHotBenchmarkCommand,
) -> Result<ThreeWayHotBenchmarkReport, ToolError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_path = command
        .workload_path
        .as_ref()
        .or(command.write_workload_path.as_ref())
        .map(|path| path.display().to_string());
    run_three_way_hot_benchmark_with_workload(command, &workload, workload_source, workload_path)
}

pub fn run_three_way_hot_benchmark_with_workload(
    command: &ThreeWayHotBenchmarkCommand,
    workload: &BenchmarkWorkload,
    workload_source: WorkloadSource,
    workload_path: Option<String>,
) -> Result<ThreeWayHotBenchmarkReport, ToolError> {
    let sqlite = Connection::open(&command.source_db, true)?;
    let concrete_line_queries = build_concrete_line_lookup_queries(
        &sqlite,
        &workload.hand_queries,
        ConcreteLineIdColumn::SourceId,
    )?;
    let memory = run_memory_workers(command, workload, &concrete_line_queries)?;
    let core = RangeStoreFacade::open_with_meta(
        &command.core_dir,
        &command.core_meta,
        command.max_open_handles,
        command.verify_checksums,
    )
    .map_err(|error| ToolError::new("THREE_WAY_CORE_OPEN", error.to_string()))?;
    let proto = ProtoRangeStoreFacade::open(
        &command.proto_root,
        command.max_open_handles,
        command.verify_checksums,
    )?;
    prewarm_dimensions(&core, &proto, workload)?;

    let mut cases = Vec::new();
    if !concrete_line_queries.is_empty() {
        cases.push(measure_concrete_lines_case(
            &core,
            &proto,
            &sqlite,
            &concrete_line_queries,
            command.warmup_iterations,
        ));
    }
    if !workload.hand_queries.is_empty() {
        cases.push(measure_hand_case(
            &core,
            &proto,
            &sqlite,
            workload,
            command.warmup_iterations,
        ));
    }
    if !workload.batch_queries.is_empty() {
        cases.push(measure_batch_case(
            &core,
            &proto,
            &sqlite,
            "batch-hand-strategy",
            &workload.batch_queries,
            command.warmup_iterations,
        ));
    }
    for (size, queries) in &workload.batch_queries_by_size {
        cases.push(measure_batch_case(
            &core,
            &proto,
            &sqlite,
            &format!("batch-size-{size}"),
            queries,
            command.warmup_iterations,
        ));
    }
    if !workload.hands_by_actions_queries.is_empty() {
        cases.push(measure_hands_by_actions_case(
            &core,
            &proto,
            &sqlite,
            &workload.hands_by_actions_queries,
            command.warmup_iterations,
        ));
    }
    if !workload.drill_scenario_queries.is_empty() {
        cases.push(measure_drill_scenarios_case(
            &core,
            &proto,
            &sqlite,
            &workload.drill_scenario_queries,
            command.warmup_iterations,
        ));
    }
    let report = ThreeWayHotBenchmarkReport {
        generated_at: generated_at_utc(),
        semantic_profile: "proto-v2-non-null-ev".to_owned(),
        source_db_path: command.source_db.display().to_string(),
        proto_storage_root: command.proto_root.display().to_string(),
        core_dir: command.core_dir.display().to_string(),
        core_meta_path: command.core_meta.display().to_string(),
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
            workload_mode: command.workload_mode,
        },
        workload: workload_summary(workload),
        workload_source: workload_source.to_string(),
        workload_path,
        totals: ThreeWayBenchmarkTotals {
            core: build_totals(&cases.iter().map(|case| case.core.clone()).collect::<Vec<_>>()),
            proto: build_totals(&cases.iter().map(|case| case.proto.clone()).collect::<Vec<_>>()),
            sqlite: build_totals(&cases.iter().map(|case| case.sqlite.clone()).collect::<Vec<_>>()),
        },
        cases,
        excluded_cases: Vec::new(),
        memory,
        notes: vec![
            "Development observation only: this runner validates result counts, not complete action payload equality; do not treat its ratios or RSS as a formal Proto / SQLite baseline.".to_owned(),
            "All three engines use the Proto V2 business profile: only action cells with hand_ev IS NOT NULL are retained.".to_owned(),
            "Core hands-by-actions applies a post-filter to its public query result because the core store still exposes NULL EV cells.".to_owned(),
            "Cases with mismatched result counts or non-zero errors are not valid performance evidence.".to_owned(),
            "Memory uses one fresh child process per engine; OS page cache can still be shared across workers.".to_owned(),
        ],
    };
    write_json_report(&command.out_path, &report)?;
    write_markdown_report(&command.md_path, render_markdown(&report))?;
    Ok(report)
}

fn load_or_create_workload(
    command: &ThreeWayHotBenchmarkCommand,
) -> Result<(BenchmarkWorkload, WorkloadSource), ToolError> {
    if let Some(path) = &command.workload_path {
        return Ok((read_workload_json(path)?, WorkloadSource::Loaded));
    }
    let workload = create_benchmark_workload(&WorkloadOptions {
        source_db_path: command.source_db.clone(),
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

pub fn run_three_way_memory_worker_from_stdin() -> Result<String, ToolError> {
    let mut input_json = String::new();
    std::io::stdin().read_to_string(&mut input_json)?;
    let input: ThreeWayMemoryWorkerInput = serde_json::from_str(&input_json)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let output = run_three_way_memory_worker(input)?;
    serde_json::to_string(&output).map_err(|error| ToolError::invalid_format(error.to_string()))
}

fn run_memory_workers(
    command: &ThreeWayHotBenchmarkCommand,
    workload: &BenchmarkWorkload,
    concrete_line_queries: &[ConcreteLineBenchmarkItem],
) -> Result<ThreeWayMemoryReport, ToolError> {
    let run = |engine| {
        run_memory_worker_process(ThreeWayMemoryWorkerInput {
            engine,
            source_db: command.source_db.clone(),
            proto_root: command.proto_root.clone(),
            core_dir: command.core_dir.clone(),
            core_meta: command.core_meta.clone(),
            workload: workload.clone(),
            concrete_line_queries: concrete_line_queries.to_vec(),
            max_open_handles: command.max_open_handles,
            verify_checksums: command.verify_checksums,
            warmup_iterations: command.warmup_iterations,
        })
    };
    let core = run(ThreeWayMemoryEngine::Core)?;
    let proto = run(ThreeWayMemoryEngine::Proto)?;
    let sqlite = run(ThreeWayMemoryEngine::Sqlite)?;
    Ok(ThreeWayMemoryReport {
        core: build_engine_memory_report(core),
        proto: build_engine_memory_report(proto),
        sqlite: build_engine_memory_report(sqlite),
        notes: vec![
            "Each engine is measured in a fresh poker-hands-storage-tools child process.".to_owned(),
            "Open is measured before any query; prewarm increment is measured after opening the requested dimensions and representative workload paths.".to_owned(),
            "RSS is process-local, but OS file page cache can still be shared between sequential workers.".to_owned(),
        ],
    })
}

fn run_memory_worker_process(
    input: ThreeWayMemoryWorkerInput,
) -> Result<ThreeWayMemoryWorkerOutput, ToolError> {
    let executable = std::env::current_exe()?;
    let input_json =
        serde_json::to_vec(&input).map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let mut child = Command::new(executable)
        .arg("three-way-memory-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| ToolError::new("THREE_WAY_MEMORY_WORKER", "worker stdin is unavailable"))?
        .write_all(&input_json)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(ToolError::new(
            "THREE_WAY_MEMORY_WORKER",
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| ToolError::new("THREE_WAY_MEMORY_WORKER", error.to_string()))
}

fn build_engine_memory_report(output: ThreeWayMemoryWorkerOutput) -> ThreeWayEngineMemoryReport {
    ThreeWayEngineMemoryReport {
        open: BenchmarkMemoryReport::new(output.before.clone(), output.after_open.clone()),
        prewarm_increment: BenchmarkMemoryReport::new(
            output.after_open.clone(),
            output.after_prewarm.clone(),
        ),
        total: BenchmarkMemoryReport::new(output.before, output.after_prewarm),
        notes: output.notes,
    }
}

fn run_three_way_memory_worker(
    input: ThreeWayMemoryWorkerInput,
) -> Result<ThreeWayMemoryWorkerOutput, ToolError> {
    let before = get_memory_snapshot();
    match input.engine {
        ThreeWayMemoryEngine::Core => {
            let service = RangeStoreFacade::open_with_meta(
                &input.core_dir,
                &input.core_meta,
                input.max_open_handles,
                input.verify_checksums,
            )
            .map_err(|error| ToolError::new("THREE_WAY_MEMORY_CORE", error.to_string()))?;
            let after_open = get_memory_snapshot();
            prewarm_core_memory(&service, &input)?;
            Ok(ThreeWayMemoryWorkerOutput {
                before,
                after_open,
                after_prewarm: get_memory_snapshot(),
                notes: vec!["Core worker uses RangeStoreFacade.".to_owned()],
            })
        }
        ThreeWayMemoryEngine::Proto => {
            let service = ProtoRangeStoreFacade::open(
                &input.proto_root,
                input.max_open_handles,
                input.verify_checksums,
            )?;
            let after_open = get_memory_snapshot();
            prewarm_proto_memory(&service, &input)?;
            Ok(ThreeWayMemoryWorkerOutput {
                before,
                after_open,
                after_prewarm: get_memory_snapshot(),
                notes: vec!["Proto worker uses ProtoRangeStoreFacade.".to_owned()],
            })
        }
        ThreeWayMemoryEngine::Sqlite => {
            let connection = Connection::open(&input.source_db, true)?;
            let after_open = get_memory_snapshot();
            prewarm_sqlite_memory(&connection, &input)?;
            Ok(ThreeWayMemoryWorkerOutput {
                before,
                after_open,
                after_prewarm: get_memory_snapshot(),
                notes: vec!["SQLite worker reads the source database directly.".to_owned()],
            })
        }
    }
}

fn prewarm_core_memory(
    service: &RangeStoreFacade,
    input: &ThreeWayMemoryWorkerInput,
) -> Result<(), ToolError> {
    for dimension in &input.workload.dimensions {
        service
            .prewarm(&parse_workload_dimension(dimension)?)
            .map_err(|error| ToolError::new("THREE_WAY_MEMORY_CORE", error.to_string()))?;
    }
    let count = input.warmup_iterations.max(1);
    for item in input.concrete_line_queries.iter().take(count) {
        core_concrete_line_count(service, item)?;
    }
    for item in input.workload.drill_scenario_queries.iter().take(count) {
        core_drill_scenario_count(service, item)?;
    }
    Ok(())
}

fn prewarm_proto_memory(
    service: &ProtoRangeStoreFacade,
    input: &ThreeWayMemoryWorkerInput,
) -> Result<(), ToolError> {
    for dimension in &input.workload.dimensions {
        service.prewarm(&parse_workload_dimension(dimension)?)?;
    }
    let count = input.warmup_iterations.max(1);
    for item in input.concrete_line_queries.iter().take(count) {
        proto_concrete_line_count(service, item)?;
    }
    for item in input.workload.drill_scenario_queries.iter().take(count) {
        proto_drill_scenario_count(service, item)?;
    }
    Ok(())
}

fn prewarm_sqlite_memory(
    connection: &Connection,
    input: &ThreeWayMemoryWorkerInput,
) -> Result<(), ToolError> {
    let count = input.warmup_iterations.max(1);
    for item in input.concrete_line_queries.iter().take(count) {
        sqlite_concrete_line_count(connection, item)?;
    }
    for item in input.workload.hand_queries.iter().take(count) {
        sqlite_hand_count(
            connection,
            &item.dimension(),
            item.concrete_line_id,
            &item.hole_cards,
        )?;
    }
    for item in input.workload.batch_queries.iter().take(count) {
        sqlite_batch_count(connection, item)?;
    }
    for item in input.workload.hands_by_actions_queries.iter().take(count) {
        sqlite_hands_by_actions_count(connection, item)?;
    }
    for item in input.workload.drill_scenario_queries.iter().take(count) {
        sqlite_drill_scenario_count(connection, item)?;
    }
    Ok(())
}

fn prewarm_dimensions(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    workload: &BenchmarkWorkload,
) -> Result<(), ToolError> {
    for value in &workload.dimensions {
        let dimension = parse_workload_dimension(value)?;
        core.prewarm(&dimension)
            .map_err(|error| ToolError::new("THREE_WAY_CORE_PREWARM", error.to_string()))?;
        proto.prewarm(&dimension)?;
    }
    Ok(())
}

fn measure_concrete_lines_case(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    sqlite: &Connection,
    queries: &[ConcreteLineBenchmarkItem],
    warmup_iterations: usize,
) -> ThreeWayBenchmarkCase {
    let name = "concrete-lines-exact";
    build_case(
        name,
        measure_benchmark_case(
            name,
            "Core exact concrete-line lookup through RangeStoreFacade.",
            queries,
            warmup_iterations,
            |item, _| core_concrete_line_count(core, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "Proto exact concrete-line lookup through per-dimension lines.db.",
            queries,
            warmup_iterations,
            |item, _| proto_concrete_line_count(proto, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "SQLite exact concrete-line lookup from the source metadata table.",
            queries,
            warmup_iterations,
            |item, _| sqlite_concrete_line_count(sqlite, item).map_err(|error| error.to_string()),
        ),
    )
}

fn measure_drill_scenarios_case(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    sqlite: &Connection,
    queries: &[DrillScenarioBenchmarkItem],
    warmup_iterations: usize,
) -> ThreeWayBenchmarkCase {
    let name = "drill-scenarios-metadata";
    build_case(
        name,
        measure_benchmark_case(
            name,
            "Core drill metadata lookup through RangeStoreFacade.",
            queries,
            warmup_iterations,
            |item, _| core_drill_scenario_count(core, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "Proto drill metadata lookup through per-dimension lines.db.",
            queries,
            warmup_iterations,
            |item, _| proto_drill_scenario_count(proto, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "SQLite drill metadata lookup from the source drill table.",
            queries,
            warmup_iterations,
            |item, _| sqlite_drill_scenario_count(sqlite, item).map_err(|error| error.to_string()),
        ),
    )
}

fn measure_hand_case(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    sqlite: &Connection,
    workload: &BenchmarkWorkload,
    warmup_iterations: usize,
) -> ThreeWayBenchmarkCase {
    let name = "hand-strategy";
    build_case(
        name,
        measure_benchmark_case(
            name,
            "Core hand strategy under Proto V2 non-NULL EV semantics.",
            &workload.hand_queries,
            warmup_iterations,
            |item, _| core_hand_count(core, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "Proto hand strategy through ProtoRangeStoreFacade.",
            &workload.hand_queries,
            warmup_iterations,
            |item, _| proto_hand_count(proto, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "SQLite hand strategy with hand_ev IS NOT NULL.",
            &workload.hand_queries,
            warmup_iterations,
            |item, _| {
                sqlite_hand_count(
                    sqlite,
                    &item.dimension(),
                    item.concrete_line_id,
                    &item.hole_cards,
                )
                .map_err(|error| error.to_string())
            },
        ),
    )
}

fn measure_batch_case(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    sqlite: &Connection,
    name: &str,
    queries: &[BatchBenchmarkItem],
    warmup_iterations: usize,
) -> ThreeWayBenchmarkCase {
    build_case(
        name,
        measure_benchmark_case(
            name,
            "Core batch strategy under Proto V2 non-NULL EV semantics.",
            queries,
            warmup_iterations,
            |item, _| core_batch_count(core, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "Proto batch strategy through ProtoRangeStoreFacade.",
            queries,
            warmup_iterations,
            |item, _| proto_batch_count(proto, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "SQLite batch strategy with hand_ev IS NOT NULL.",
            queries,
            warmup_iterations,
            |item, _| sqlite_batch_count(sqlite, item).map_err(|error| error.to_string()),
        ),
    )
}

fn measure_hands_by_actions_case(
    core: &RangeStoreFacade,
    proto: &ProtoRangeStoreFacade,
    sqlite: &Connection,
    queries: &[HandsByActionsBenchmarkItem],
    warmup_iterations: usize,
) -> ThreeWayBenchmarkCase {
    let name = "hands-by-actions";
    build_case(
        name,
        measure_benchmark_case(
            name,
            "Core hands-by-actions with a Proto V2 non-NULL EV post-filter.",
            queries,
            warmup_iterations,
            |item, _| core_hands_by_actions_count(core, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "Proto hands-by-actions through ProtoRangeStoreFacade.",
            queries,
            warmup_iterations,
            |item, _| proto_hands_by_actions_count(proto, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            name,
            "SQLite hands-by-actions with hand_ev IS NOT NULL.",
            queries,
            warmup_iterations,
            |item, _| {
                sqlite_hands_by_actions_count(sqlite, item).map_err(|error| error.to_string())
            },
        ),
    )
}

fn build_case(
    name: &str,
    core: BenchmarkCaseResult,
    proto: BenchmarkCaseResult,
    sqlite: BenchmarkCaseResult,
) -> ThreeWayBenchmarkCase {
    ThreeWayBenchmarkCase {
        name: name.to_owned(),
        core_to_sqlite_avg_latency_ratio: safe_ratio(core.avg_ms, sqlite.avg_ms),
        proto_to_sqlite_avg_latency_ratio: safe_ratio(proto.avg_ms, sqlite.avg_ms),
        proto_to_core_avg_latency_ratio: safe_ratio(proto.avg_ms, core.avg_ms),
        core_to_sqlite_p95_latency_ratio: safe_ratio(core.p95_ms, sqlite.p95_ms),
        proto_to_sqlite_p95_latency_ratio: safe_ratio(proto.p95_ms, sqlite.p95_ms),
        proto_to_core_p95_latency_ratio: safe_ratio(proto.p95_ms, core.p95_ms),
        result_count_match: core.result_count == proto.result_count
            && proto.result_count == sqlite.result_count,
        core,
        proto,
        sqlite,
    }
}

fn core_hand_count(
    service: &RangeStoreFacade,
    item: &crate::benchmark::types::HandBenchmarkItem,
) -> Result<usize, ToolError> {
    service
        .query_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)
        .map(|result| {
            result
                .actions
                .iter()
                .filter(|action| action.hand_ev.is_some())
                .count()
        })
        .map_err(|error| ToolError::new("THREE_WAY_CORE_QUERY", error.to_string()))
}

fn proto_hand_count(
    service: &ProtoRangeStoreFacade,
    item: &crate::benchmark::types::HandBenchmarkItem,
) -> Result<usize, ToolError> {
    service
        .query_hand_strategy(&item.dimension(), item.concrete_line_id, &item.hole_cards)
        .map(|result| result.actions.len())
}

fn sqlite_hand_count(
    connection: &Connection,
    dimension: &DimensionRef,
    concrete_line_id: u32,
    hole_cards: &str,
) -> Result<usize, ToolError> {
    let table = quote_identifier(&range_table_name(dimension))?;
    let sql = format!(
        "SELECT action_name, action_size, amount_bb, frequency, hand_ev
         FROM {table}
         WHERE concrete_line_id = ?1 AND hole_cards = ?2 AND hand_ev IS NOT NULL"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[Value::from(concrete_line_id), Value::from(hole_cards)])?;
    let mut count = 0;
    while statement.step_row()? {
        let _action_name = statement.column_text(0)?;
        let _action_size = statement.column_f64(1);
        let _amount_bb = statement.column_f64(2);
        let _frequency = statement.column_f64(3);
        let _hand_ev = statement.column_f64(4);
        count += 1;
    }
    Ok(count)
}

fn core_concrete_line_count(
    service: &RangeStoreFacade,
    item: &ConcreteLineBenchmarkItem,
) -> Result<usize, ToolError> {
    let lines = service
        .get_concrete_lines(
            &item.dimension(),
            ConcreteLineFilter::Concrete(&item.concrete_line),
        )
        .map_err(|error| ToolError::new("THREE_WAY_CORE_CONCRETE_LINE", error.to_string()))?;
    validate_concrete_line_id(&lines, item.concrete_line_id)
}

fn proto_concrete_line_count(
    service: &ProtoRangeStoreFacade,
    item: &ConcreteLineBenchmarkItem,
) -> Result<usize, ToolError> {
    let lines = service.get_concrete_lines(
        &item.dimension(),
        ConcreteLineFilter::Concrete(&item.concrete_line),
    )?;
    validate_concrete_line_id(&lines, item.concrete_line_id)
}

fn sqlite_concrete_line_count(
    connection: &Connection,
    item: &ConcreteLineBenchmarkItem,
) -> Result<usize, ToolError> {
    let table = quote_identifier(&concrete_lines_table_name(&item.dimension()))?;
    let sql = format!(
        "SELECT id
         FROM {table}
         WHERE concrete_line = ?1
         ORDER BY id"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[Value::from(item.concrete_line.as_str())])?;
    let mut ids = Vec::new();
    while statement.step_row()? {
        ids.push(statement.column_u32(0)?);
    }
    if ids.len() != 1 || ids[0] != item.concrete_line_id {
        return Err(ToolError::new(
            "THREE_WAY_SQLITE_CONCRETE_LINE",
            format!(
                "expected concrete_line_id={}, got {:?}",
                item.concrete_line_id, ids
            ),
        ));
    }
    Ok(ids.len())
}

fn validate_concrete_line_id(
    lines: &[range_store_core::metadata::ConcreteLineRow],
    expected_id: u32,
) -> Result<usize, ToolError> {
    if lines.len() != 1 || lines[0].concrete_line_id != expected_id {
        return Err(ToolError::new(
            "THREE_WAY_CONCRETE_LINE",
            format!(
                "expected concrete_line_id={expected_id}, got {:?}",
                lines
                    .iter()
                    .map(|line| line.concrete_line_id)
                    .collect::<Vec<_>>()
            ),
        ));
    }
    Ok(lines.len())
}

fn core_drill_scenario_count(
    service: &RangeStoreFacade,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    service
        .get_drill_scenario_lines(
            &item.strategy,
            &item.drill_name,
            item.player_count,
            item.drill_depth,
        )
        .map(|lines| lines.len())
        .map_err(|error| ToolError::new("THREE_WAY_CORE_DRILL", error.to_string()))
}

fn proto_drill_scenario_count(
    service: &ProtoRangeStoreFacade,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    service
        .get_drill_scenario_lines(
            &item.strategy,
            &item.drill_name,
            item.player_count,
            item.drill_depth,
        )
        .map(|lines| lines.len())
}

fn sqlite_drill_scenario_count(
    connection: &Connection,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    let raw_table = drill_scenario_table_name(&item.strategy);
    let depth_column = drill_depth_column(connection, &raw_table)?.ok_or_else(|| {
        ToolError::invalid_format(format!(
            "Drill scenario table {raw_table} must contain depth or drill_depth"
        ))
    })?;
    let table = quote_identifier(&raw_table)?;
    let sql = format!(
        "SELECT DISTINCT abstract_line
         FROM {table}
         WHERE drill_name = ?1 AND player_count = ?2 AND {depth_column} = ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[
        Value::from(item.drill_name.as_str()),
        Value::from(item.player_count),
        Value::from(item.drill_depth),
    ])?;
    let mut lines = Vec::new();
    while statement.step_row()? {
        lines.push(statement.column_text(0)?);
    }
    Ok(lines.len())
}

fn core_batch_count(
    service: &RangeStoreFacade,
    item: &BatchBenchmarkItem,
) -> Result<usize, ToolError> {
    let requests = item
        .requests
        .iter()
        .map(|request| (request.concrete_line_id, request.hole_cards.clone()))
        .collect::<Vec<_>>();
    service
        .query_batch(&item.dimension(), &requests)
        .map(|result| {
            result
                .results
                .iter()
                .map(|entry| {
                    entry
                        .actions
                        .iter()
                        .filter(|action| action.hand_ev.is_some())
                        .count()
                })
                .sum()
        })
        .map_err(|error| ToolError::new("THREE_WAY_CORE_BATCH", error.to_string()))
}

fn proto_batch_count(
    service: &ProtoRangeStoreFacade,
    item: &BatchBenchmarkItem,
) -> Result<usize, ToolError> {
    let requests = item
        .requests
        .iter()
        .map(|request| (request.concrete_line_id, request.hole_cards.clone()))
        .collect::<Vec<_>>();
    service
        .query_batch(&item.dimension(), &requests)
        .map(|result| result.results.iter().map(|entry| entry.actions.len()).sum())
}

fn sqlite_batch_count(
    connection: &Connection,
    item: &BatchBenchmarkItem,
) -> Result<usize, ToolError> {
    item.requests.iter().try_fold(0, |count, request| {
        sqlite_hand_count(
            connection,
            &item.dimension(),
            request.concrete_line_id,
            &request.hole_cards,
        )
        .map(|value| count + value)
    })
}

fn core_hands_by_actions_count(
    service: &RangeStoreFacade,
    item: &HandsByActionsBenchmarkItem,
) -> Result<usize, ToolError> {
    let dimension = item.dimension();
    let filters = parse_action_filters(item.actions.clone())
        .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
    let frequency_filter = FrequencyFilter::from_request(item.frequency);
    let candidates = service
        .hands_by_actions(&dimension, item.concrete_line_id, &[], Some(-1.0))
        .map_err(|error| ToolError::new("THREE_WAY_CORE_HANDS_BY_ACTIONS", error.to_string()))?;
    let mut count = 0;
    for hand in candidates {
        let strategy = service
            .query_hand_strategy(&dimension, item.concrete_line_id, &hand)
            .map_err(|error| {
                ToolError::new("THREE_WAY_CORE_HANDS_BY_ACTIONS", error.to_string())
            })?;
        if strategy.actions.iter().any(|action| {
            action.hand_ev.is_some()
                && frequency_filter.matches(proto_v2_frequency(action.frequency))
                && (filters.is_empty()
                    || filters.iter().any(|filter| {
                        action_matches_filter(action.action_name.as_str(), action.amount_bb, filter)
                    }))
        }) {
            count += 1;
        }
    }
    Ok(count)
}

fn proto_hands_by_actions_count(
    service: &ProtoRangeStoreFacade,
    item: &HandsByActionsBenchmarkItem,
) -> Result<usize, ToolError> {
    let filters = parse_action_filters(item.actions.clone())
        .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
    service
        .query_hands_by_actions(
            &item.dimension(),
            item.concrete_line_id,
            &filters,
            item.frequency,
        )
        .map(|hands| hands.len())
}

fn sqlite_hands_by_actions_count(
    connection: &Connection,
    item: &HandsByActionsBenchmarkItem,
) -> Result<usize, ToolError> {
    let table = quote_identifier(&range_table_name(&item.dimension()))?;
    let threshold = item.frequency.unwrap_or(DEFAULT_HANDS_BY_ACTIONS_FREQUENCY);
    let mut values = vec![Value::from(item.concrete_line_id), Value::from(threshold)];
    let sql = if item.actions.is_empty() {
        format!("SELECT COUNT(DISTINCT hole_cards) FROM {table} WHERE concrete_line_id = ?1 AND ROUND(frequency * 10000.0) / 10000.0 > ?2 AND hand_ev IS NOT NULL")
    } else {
        let filters = parse_action_filters(item.actions.clone())
            .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
        let mut clauses = Vec::with_capacity(filters.len());
        for filter in filters {
            let action_parameter = values.len() + 1;
            values.push(Value::from(filter.action_name.as_str()));
            let amount = if let Some(amount_bb) = filter.amount_bb {
                let amount_parameter = values.len() + 1;
                values.push(Value::from(f64::from(amount_bb)));
                format!(" AND ABS(ROUND(amount_bb * 100.0) / 100.0 - ?{amount_parameter}) <= {ACTION_AMOUNT_TOLERANCE}")
            } else {
                String::new()
            };
            clauses.push(format!("(action_name = ?{action_parameter}{amount})"));
        }
        format!("SELECT COUNT(DISTINCT hole_cards) FROM {table} WHERE concrete_line_id = ?1 AND ROUND(frequency * 10000.0) / 10000.0 > ?2 AND hand_ev IS NOT NULL AND ({})", clauses.join(" OR "))
    };
    let mut statement = connection.prepare(&sql)?;
    statement.start(&values)?;
    Ok(if statement.step_row()? {
        usize::try_from(statement.column_i64(0)).unwrap_or_default()
    } else {
        0
    })
}

fn action_matches_filter(action_name: &str, amount_bb: f32, filter: &ActionFilter) -> bool {
    action_name == filter.action_name.as_str()
        && filter.amount_bb.is_none_or(|amount| {
            let proto_amount_bb = (amount_bb * 100.0).round() / 100.0;
            (proto_amount_bb - amount).abs() <= f32::EPSILON
        })
}

fn proto_v2_frequency(frequency: f64) -> f64 {
    (frequency * 10_000.0).round() / 10_000.0
}

fn parse_workload_dimension(value: &str) -> Result<DimensionRef, ToolError> {
    let parts = value.split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(ToolError::invalid_argument(format!(
            "Invalid workload dimension: {value}"
        )));
    }
    let player_count = parts[1]
        .strip_suffix("max")
        .unwrap_or(parts[1])
        .parse()
        .map_err(|_| ToolError::invalid_argument(format!("Invalid workload dimension: {value}")))?;
    let depth_bb = parts[2]
        .strip_suffix("BB")
        .unwrap_or(parts[2])
        .parse()
        .map_err(|_| ToolError::invalid_argument(format!("Invalid workload dimension: {value}")))?;
    Ok(DimensionRef::new(parts[0], player_count, depth_bb))
}

fn workload_summary(workload: &BenchmarkWorkload) -> BenchmarkWorkloadSummary {
    BenchmarkWorkloadSummary {
        dimensions: workload.dimensions.clone(),
        hand_queries: workload.hand_queries.len(),
        batch_queries: workload.batch_queries.len(),
        batch_size: workload.batch_size,
        hands_by_actions_queries: workload.hands_by_actions_queries.len(),
        drill_scenario_queries: workload.drill_scenario_queries.len(),
    }
}

fn render_markdown(report: &ThreeWayHotBenchmarkReport) -> String {
    let mut markdown = String::from(
        "# Core vs Proto vs SQLite Development Observation (Not a Formal Baseline)\n\n",
    );
    markdown.push_str(&format!("Generated at: {}\n\n", report.generated_at));
    markdown.push_str(
        "> This runner only checks result counts and uses non-equivalent cache and memory profiles. Do not use these ratios or RSS values as Proto / SQLite performance evidence.\n\n",
    );
    markdown.push_str(&format!(
        "- Semantic profile: `{}`\n",
        report.semantic_profile
    ));
    markdown.push_str(&format!(
        "- Proto storage root: `{}`\n\n",
        report.proto_storage_root
    ));
    markdown.push_str("## Shared Strategy Queries\n\n");
    let rows = report
        .cases
        .iter()
        .map(|case| {
            vec![
                case.name.clone(),
                format!("{:.3}", case.core.avg_ms),
                format!("{:.3}", case.proto.avg_ms),
                format!("{:.3}", case.sqlite.avg_ms),
                format!("{:.2}", case.proto_to_core_avg_latency_ratio),
                format!("{:.2}", case.proto_to_sqlite_avg_latency_ratio),
                format!("{:.2}", case.proto_to_core_p95_latency_ratio),
                format!("{:.2}", case.proto_to_sqlite_p95_latency_ratio),
                case.result_count_match.to_string(),
                format!(
                    "{}/{}/{}",
                    case.core.error_count, case.proto.error_count, case.sqlite.error_count
                ),
            ]
        })
        .collect::<Vec<_>>();
    markdown.push_str(&markdown_table(
        &[
            "case",
            "core avg ms",
            "proto avg ms",
            "sqlite avg ms",
            "proto/core avg",
            "proto/sqlite avg",
            "proto/core p95",
            "proto/sqlite p95",
            "result count match",
            "errors C/P/S",
        ],
        &rows,
    ));
    markdown.push_str("\nRatios are descriptive development observations only.\n\n");
    if !report.excluded_cases.is_empty() {
        markdown.push_str("## Excluded Cases\n\n");
        for case in &report.excluded_cases {
            markdown.push_str(&format!("- `{}`: {}\n", case.name, case.reason));
        }
    }
    markdown.push_str("\n## Memory (Separate Workers)\n\n");
    let memory_rows = [
        ("Core", &report.memory.core),
        ("Proto", &report.memory.proto),
        ("SQLite", &report.memory.sqlite),
    ]
    .into_iter()
    .map(|(engine, memory)| {
        vec![
            engine.to_owned(),
            format_optional_signed_bytes(memory.open.delta_rss_bytes),
            format_optional_signed_bytes(memory.prewarm_increment.delta_rss_bytes),
            format_optional_signed_bytes(memory.total.delta_rss_bytes),
        ]
    })
    .collect::<Vec<_>>();
    markdown.push_str(&markdown_table(
        &[
            "engine",
            "open RSS delta",
            "prewarm RSS delta",
            "total RSS delta",
        ],
        &memory_rows,
    ));
    for note in &report.memory.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown.push_str("\n## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn format_optional_signed_bytes(value: Option<i64>) -> String {
    match value {
        Some(value) if value < 0 => format!("-{}", format_binary_bytes(value.unsigned_abs())),
        Some(value) => format_binary_bytes(value as u64),
        None => "n/a".to_owned(),
    }
}
