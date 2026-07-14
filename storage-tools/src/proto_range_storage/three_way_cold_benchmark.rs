use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;

use range_store_core::dimension::{quote_identifier, DimensionRef, DimensionSpec};
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::query::RangeStoreFacade;
use range_store_core::sqlite::{Connection, Value};
use serde::{Deserialize, Serialize};

use crate::benchmark::cli::{next_value, parse_u32, parse_usize};
use crate::benchmark::cold::types::LatencySummary;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, MemorySnapshot};
use crate::benchmark::report_support::{
    format_binary_bytes, format_ms, generated_at_utc, markdown_table, write_json_report,
    write_markdown_report,
};
use crate::benchmark::types::{drill_scenario_table_name, range_table_name};
use crate::benchmark::workload::drill_depth_column;
use crate::errors::ToolError;

use super::query_facade::ProtoRangeStoreFacade;

#[derive(Debug, Clone)]
pub struct ThreeWayColdBenchmarkCommand {
    pub source_db: PathBuf,
    pub proto_root: PathBuf,
    pub core_dir: PathBuf,
    pub core_meta: PathBuf,
    pub dimension: DimensionSpec,
    pub query: ThreeWayColdQuery,
    pub runs: usize,
    pub max_open_handles: usize,
    pub verify_checksums: bool,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayColdBenchmarkReport {
    pub generated_at: String,
    pub mode: String,
    pub source_db_path: String,
    pub proto_storage_root: String,
    pub core_dir: String,
    pub dimension: String,
    pub operation: String,
    pub query: String,
    pub runs_per_engine: usize,
    pub core: ThreeWayColdEngineReport,
    pub proto: ThreeWayColdEngineReport,
    pub sqlite: ThreeWayColdEngineReport,
    pub notes: Vec<String>,
}

impl ThreeWayColdBenchmarkReport {
    pub fn has_errors(&self) -> bool {
        self.core.error_count > 0 || self.proto.error_count > 0 || self.sqlite.error_count > 0
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayColdEngineReport {
    pub successful_runs: usize,
    pub error_count: usize,
    pub result_count: u64,
    pub open_ms: LatencySummary,
    pub prewarm_ms: LatencySummary,
    pub first_query_ms: LatencySummary,
    pub total_ms: LatencySummary,
    pub memory: ThreeWayColdMemorySummary,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayColdMemorySummary {
    pub open_rss_bytes: LatencySummary,
    pub prewarm_rss_bytes: LatencySummary,
    pub first_query_rss_bytes: LatencySummary,
    pub total_rss_bytes: LatencySummary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThreeWayColdEngine {
    Core,
    Proto,
    Sqlite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case")]
pub enum ThreeWayColdQuery {
    HandStrategy {
        concrete_line_id: u32,
        hand: String,
    },
    DrillScenariosMetadata {
        drill_name: String,
        drill_depth: u32,
    },
}

impl ThreeWayColdQuery {
    fn operation_name(&self) -> &'static str {
        match self {
            Self::HandStrategy { .. } => "hand-strategy",
            Self::DrillScenariosMetadata { .. } => "drill-scenarios-metadata",
        }
    }

    fn display(&self, player_count: u32) -> String {
        match self {
            Self::HandStrategy {
                concrete_line_id,
                hand,
            } => format!("line {concrete_line_id} / {hand}"),
            Self::DrillScenariosMetadata {
                drill_name,
                drill_depth,
            } => format!("{drill_name} / {player_count}max / {drill_depth}BB"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreeWayColdWorkerInput {
    engine: ThreeWayColdEngine,
    source_db: PathBuf,
    proto_root: PathBuf,
    core_dir: PathBuf,
    core_meta: PathBuf,
    dimension: ThreeWayColdWorkerDimension,
    query: ThreeWayColdQuery,
    max_open_handles: usize,
    verify_checksums: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreeWayColdWorkerDimension {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
}

impl From<&DimensionRef> for ThreeWayColdWorkerDimension {
    fn from(value: &DimensionRef) -> Self {
        Self {
            strategy: value.strategy.clone(),
            player_count: value.player_count,
            depth_bb: value.depth_bb,
        }
    }
}

impl ThreeWayColdWorkerInput {
    fn dimension_ref(&self) -> DimensionRef {
        DimensionRef::new(
            self.dimension.strategy.clone(),
            self.dimension.player_count,
            self.dimension.depth_bb,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreeWayColdWorkerOutput {
    pub ok: bool,
    pub result_count: usize,
    pub before: MemorySnapshot,
    pub after_open: MemorySnapshot,
    pub after_prewarm: MemorySnapshot,
    pub after_first_query: MemorySnapshot,
    pub open_ms: f64,
    pub prewarm_ms: f64,
    pub first_query_ms: f64,
    pub total_ms: f64,
    pub error: Option<String>,
}

pub fn parse_three_way_cold_benchmark_args(
    args: Vec<String>,
) -> Result<ThreeWayColdBenchmarkCommand, ToolError> {
    let mut source_db = None;
    let mut proto_root = None;
    let mut core_dir = None;
    let mut core_meta = None;
    let mut dimension = None;
    let mut concrete_line_id = None;
    let mut hand_id = None;
    let mut operation = "hand-strategy".to_owned();
    let mut drill_name = None;
    let mut drill_depth = None;
    let mut runs = 20usize;
    let mut max_open_handles = 1usize;
    let mut verify_checksums = false;
    let mut out_path = PathBuf::from("reports/benchmark-core-proto-sqlite-cold.json");
    let mut md_path = PathBuf::from("reports/benchmark-core-proto-sqlite-cold.md");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--proto-root" => proto_root = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--core-dir" => core_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--core-meta" => core_meta = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--hand-id" => hand_id = Some(parse_u32("--hand-id", next_value(&args, &mut index)?)?),
            "--operation" => operation = next_value(&args, &mut index)?.to_owned(),
            "--drill-name" => drill_name = Some(next_value(&args, &mut index)?.to_owned()),
            "--drill-depth" => {
                drill_depth = Some(parse_u32("--drill-depth", next_value(&args, &mut index)?)?)
            }
            "--runs" => runs = parse_usize("--runs", next_value(&args, &mut index)?)?,
            "--max-open-handles" => {
                max_open_handles =
                    parse_usize("--max-open-handles", next_value(&args, &mut index)?)?
            }
            "--verify-checksum" => verify_checksums = true,
            "--out" => out_path = PathBuf::from(next_value(&args, &mut index)?),
            "--md" => md_path = PathBuf::from(next_value(&args, &mut index)?),
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown benchmark-three-way-cold option: {option}"
                )))
            }
        }
        index += 1;
    }
    let core_dir = core_dir.ok_or_else(|| ToolError::invalid_argument("--core-dir is required"))?;
    if runs == 0 || max_open_handles == 0 {
        return Err(ToolError::invalid_argument(
            "--runs and --max-open-handles must be at least 1",
        ));
    }
    let query = match operation.as_str() {
        "hand-strategy" => {
            let hand_id = hand_id.ok_or_else(|| {
                ToolError::invalid_argument("--hand-id is required for hand-strategy")
            })?;
            if hand_id >= 169 {
                return Err(ToolError::invalid_argument("--hand-id must be in 0..168"));
            }
            ThreeWayColdQuery::HandStrategy {
                concrete_line_id: concrete_line_id.ok_or_else(|| {
                    ToolError::invalid_argument("--concrete-line-id is required for hand-strategy")
                })?,
                hand: hand_code_from_id(hand_id as u8),
            }
        }
        "drill-scenarios-metadata" => ThreeWayColdQuery::DrillScenariosMetadata {
            drill_name: drill_name.ok_or_else(|| {
                ToolError::invalid_argument("--drill-name is required for drill-scenarios-metadata")
            })?,
            drill_depth: drill_depth.ok_or_else(|| {
                ToolError::invalid_argument(
                    "--drill-depth is required for drill-scenarios-metadata",
                )
            })?,
        },
        _ => {
            return Err(ToolError::invalid_argument(
                "--operation must be hand-strategy or drill-scenarios-metadata",
            ))
        }
    };
    Ok(ThreeWayColdBenchmarkCommand {
        source_db: source_db.ok_or_else(|| ToolError::invalid_argument("--source is required"))?,
        proto_root: proto_root
            .ok_or_else(|| ToolError::invalid_argument("--proto-root is required"))?,
        core_meta: core_meta.unwrap_or_else(|| core_dir.join("meta.db")),
        core_dir,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        query,
        runs,
        max_open_handles,
        verify_checksums,
        out_path,
        md_path,
    })
}

pub fn run_three_way_cold_benchmark(
    command: &ThreeWayColdBenchmarkCommand,
) -> Result<ThreeWayColdBenchmarkReport, ToolError> {
    let dimension = DimensionRef::new(
        &command.dimension.strategy,
        command.dimension.player_count,
        command.dimension.depth_bb,
    );
    let run = |engine| {
        let mut results = Vec::with_capacity(command.runs);
        for _ in 0..command.runs {
            results.push(run_worker_process(ThreeWayColdWorkerInput {
                engine,
                source_db: command.source_db.clone(),
                proto_root: command.proto_root.clone(),
                core_dir: command.core_dir.clone(),
                core_meta: command.core_meta.clone(),
                dimension: ThreeWayColdWorkerDimension::from(&dimension),
                query: command.query.clone(),
                max_open_handles: command.max_open_handles,
                verify_checksums: command.verify_checksums,
            })?);
        }
        Ok::<_, ToolError>(summarize_engine(&results))
    };
    let report = ThreeWayColdBenchmarkReport {
        generated_at: generated_at_utc(),
        mode: "process-cold".to_owned(),
        source_db_path: command.source_db.display().to_string(),
        proto_storage_root: command.proto_root.display().to_string(),
        core_dir: command.core_dir.display().to_string(),
        dimension: format!(
            "{}:{}max:{}BB",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        ),
        operation: command.query.operation_name().to_owned(),
        query: command.query.display(dimension.player_count),
        runs_per_engine: command.runs,
        core: run(ThreeWayColdEngine::Core)?,
        proto: run(ThreeWayColdEngine::Proto)?,
        sqlite: run(ThreeWayColdEngine::Sqlite)?,
        notes: vec![
            "Each run starts a new worker process; process-cold refreshes process state but does not evict the OS page cache.".to_owned(),
            "Hand-strategy prewarms the requested Core and Proto matrix dimension. Drill metadata does not prewarm a matrix; SQLite has no equivalent reader prewarm.".to_owned(),
            "RSS deltas are measured from immediately before each engine opens its store or database connection.".to_owned(),
        ],
    };
    write_json_report(&command.out_path, &report)?;
    write_markdown_report(&command.md_path, render_markdown(&report))?;
    Ok(report)
}

pub fn run_three_way_cold_worker_from_stdin() -> Result<ThreeWayColdWorkerOutput, ToolError> {
    let mut json = String::new();
    std::io::stdin().read_to_string(&mut json)?;
    let input = serde_json::from_str(&json)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    Ok(run_worker(input))
}

fn run_worker_process(
    input: ThreeWayColdWorkerInput,
) -> Result<ThreeWayColdWorkerOutput, ToolError> {
    let input_json =
        serde_json::to_vec(&input).map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("three-way-cold-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| ToolError::new("THREE_WAY_COLD_WORKER", "worker stdin is unavailable"))?
        .write_all(&input_json)?;
    let output = child.wait_with_output()?;
    let parsed: ThreeWayColdWorkerOutput = serde_json::from_slice(&output.stdout)
        .map_err(|error| ToolError::new("THREE_WAY_COLD_WORKER", error.to_string()))?;
    if !output.status.success() && parsed.error.is_none() {
        return Err(ToolError::new(
            "THREE_WAY_COLD_WORKER",
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(parsed)
}

fn run_worker(input: ThreeWayColdWorkerInput) -> ThreeWayColdWorkerOutput {
    let before = get_memory_snapshot();
    let started = Instant::now();
    let result = match input.engine {
        ThreeWayColdEngine::Core => run_core_worker(&input, &before, started),
        ThreeWayColdEngine::Proto => run_proto_worker(&input, &before, started),
        ThreeWayColdEngine::Sqlite => run_sqlite_worker(&input, &before, started),
    };
    result.unwrap_or_else(|error| failed_worker(before, started, error))
}

fn run_core_worker(
    input: &ThreeWayColdWorkerInput,
    before: &MemorySnapshot,
    started: Instant,
) -> Result<ThreeWayColdWorkerOutput, ToolError> {
    let open = Instant::now();
    let service = RangeStoreFacade::open_with_meta(
        &input.core_dir,
        &input.core_meta,
        input.max_open_handles,
        input.verify_checksums,
    )
    .map_err(|error| ToolError::new("THREE_WAY_COLD_CORE", error.to_string()))?;
    let open_ms = elapsed_ms(open);
    let after_open = get_memory_snapshot();
    let prewarm_ms = match input.query {
        ThreeWayColdQuery::HandStrategy { .. } => {
            let prewarm = Instant::now();
            service
                .prewarm(&input.dimension_ref())
                .map_err(|error| ToolError::new("THREE_WAY_COLD_CORE", error.to_string()))?;
            elapsed_ms(prewarm)
        }
        ThreeWayColdQuery::DrillScenariosMetadata { .. } => 0.0,
    };
    let after_prewarm = get_memory_snapshot();
    let query = Instant::now();
    let result_count = match &input.query {
        ThreeWayColdQuery::HandStrategy {
            concrete_line_id,
            hand,
        } => service
            .query_hand_strategy(&input.dimension_ref(), *concrete_line_id, hand)
            .map_err(|error| ToolError::new("THREE_WAY_COLD_CORE", error.to_string()))?
            .actions
            .iter()
            .filter(|action| action.hand_ev.is_some())
            .count(),
        ThreeWayColdQuery::DrillScenariosMetadata {
            drill_name,
            drill_depth,
        } => service
            .get_drill_scenario_lines(
                &input.dimension.strategy,
                drill_name,
                input.dimension.player_count,
                *drill_depth,
            )
            .map_err(|error| ToolError::new("THREE_WAY_COLD_CORE", error.to_string()))?
            .len(),
    };
    Ok(successful_worker(
        result_count,
        before.clone(),
        after_open,
        after_prewarm,
        elapsed_ms(query),
        open_ms,
        prewarm_ms,
        started,
    ))
}

fn run_proto_worker(
    input: &ThreeWayColdWorkerInput,
    before: &MemorySnapshot,
    started: Instant,
) -> Result<ThreeWayColdWorkerOutput, ToolError> {
    let open = Instant::now();
    let service = ProtoRangeStoreFacade::open(
        &input.proto_root,
        input.max_open_handles,
        input.verify_checksums,
    )?;
    let open_ms = elapsed_ms(open);
    let after_open = get_memory_snapshot();
    let prewarm_ms = match input.query {
        ThreeWayColdQuery::HandStrategy { .. } => {
            let prewarm = Instant::now();
            service.prewarm(&input.dimension_ref())?;
            elapsed_ms(prewarm)
        }
        ThreeWayColdQuery::DrillScenariosMetadata { .. } => 0.0,
    };
    let after_prewarm = get_memory_snapshot();
    let query = Instant::now();
    let result_count = match &input.query {
        ThreeWayColdQuery::HandStrategy {
            concrete_line_id,
            hand,
        } => service
            .query_hand_strategy(&input.dimension_ref(), *concrete_line_id, hand)?
            .actions
            .len(),
        ThreeWayColdQuery::DrillScenariosMetadata {
            drill_name,
            drill_depth,
        } => service
            .get_drill_scenario_lines(
                &input.dimension.strategy,
                drill_name,
                input.dimension.player_count,
                *drill_depth,
            )?
            .len(),
    };
    Ok(successful_worker(
        result_count,
        before.clone(),
        after_open,
        after_prewarm,
        elapsed_ms(query),
        open_ms,
        prewarm_ms,
        started,
    ))
}

fn run_sqlite_worker(
    input: &ThreeWayColdWorkerInput,
    before: &MemorySnapshot,
    started: Instant,
) -> Result<ThreeWayColdWorkerOutput, ToolError> {
    let open = Instant::now();
    let connection = Connection::open(&input.source_db, true)?;
    let open_ms = elapsed_ms(open);
    let after_open = get_memory_snapshot();
    let after_prewarm = get_memory_snapshot();
    let query = Instant::now();
    let result_count = match &input.query {
        ThreeWayColdQuery::HandStrategy { .. } => sqlite_hand_count(&connection, input)?,
        ThreeWayColdQuery::DrillScenariosMetadata { .. } => sqlite_drill_count(&connection, input)?,
    };
    Ok(successful_worker(
        result_count,
        before.clone(),
        after_open,
        after_prewarm,
        elapsed_ms(query),
        open_ms,
        0.0,
        started,
    ))
}

fn sqlite_hand_count(
    connection: &Connection,
    input: &ThreeWayColdWorkerInput,
) -> Result<usize, ToolError> {
    let ThreeWayColdQuery::HandStrategy {
        concrete_line_id,
        hand,
    } = &input.query
    else {
        unreachable!("hand query required")
    };
    let table = quote_identifier(&range_table_name(&input.dimension_ref()))?;
    let sql = format!(
        "SELECT COUNT(*) FROM {table}
         WHERE concrete_line_id = ?1 AND hole_cards = ?2 AND hand_ev IS NOT NULL"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[Value::from(*concrete_line_id), Value::from(hand.as_str())])?;
    Ok(if statement.step_row()? {
        usize::try_from(statement.column_i64(0)).unwrap_or_default()
    } else {
        0
    })
}

fn sqlite_drill_count(
    connection: &Connection,
    input: &ThreeWayColdWorkerInput,
) -> Result<usize, ToolError> {
    let ThreeWayColdQuery::DrillScenariosMetadata {
        drill_name,
        drill_depth,
    } = &input.query
    else {
        unreachable!("drill query required")
    };
    let raw_table = drill_scenario_table_name(&input.dimension.strategy);
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
        Value::from(drill_name.as_str()),
        Value::from(input.dimension.player_count),
        Value::from(*drill_depth),
    ])?;
    let mut lines = Vec::new();
    while statement.step_row()? {
        lines.push(statement.column_text(0)?);
    }
    Ok(lines.len())
}

fn successful_worker(
    result_count: usize,
    before: MemorySnapshot,
    after_open: MemorySnapshot,
    after_prewarm: MemorySnapshot,
    first_query_ms: f64,
    open_ms: f64,
    prewarm_ms: f64,
    started: Instant,
) -> ThreeWayColdWorkerOutput {
    ThreeWayColdWorkerOutput {
        ok: true,
        result_count,
        before,
        after_open,
        after_prewarm,
        after_first_query: get_memory_snapshot(),
        open_ms,
        prewarm_ms,
        first_query_ms,
        total_ms: elapsed_ms(started),
        error: None,
    }
}

fn failed_worker(
    before: MemorySnapshot,
    started: Instant,
    error: ToolError,
) -> ThreeWayColdWorkerOutput {
    ThreeWayColdWorkerOutput {
        ok: false,
        result_count: 0,
        after_open: before.clone(),
        after_prewarm: before.clone(),
        after_first_query: before.clone(),
        before,
        open_ms: 0.0,
        prewarm_ms: 0.0,
        first_query_ms: 0.0,
        total_ms: elapsed_ms(started),
        error: Some(error.to_string()),
    }
}

fn summarize_engine(results: &[ThreeWayColdWorkerOutput]) -> ThreeWayColdEngineReport {
    let successful = results
        .iter()
        .filter(|result| result.ok)
        .collect::<Vec<_>>();
    let values = |select: fn(&ThreeWayColdWorkerOutput) -> f64| {
        LatencySummary::from_values(
            &successful
                .iter()
                .map(|result| select(result))
                .collect::<Vec<_>>(),
        )
    };
    let memory = |before: fn(&ThreeWayColdWorkerOutput) -> &MemorySnapshot,
                  after: fn(&ThreeWayColdWorkerOutput) -> &MemorySnapshot| {
        LatencySummary::from_values(
            &successful
                .iter()
                .filter_map(|result| {
                    rss_delta(before(result), after(result)).map(|value| value as f64)
                })
                .collect::<Vec<_>>(),
        )
    };
    ThreeWayColdEngineReport {
        successful_runs: successful.len(),
        error_count: results.len() - successful.len(),
        result_count: successful
            .iter()
            .map(|result| result.result_count as u64)
            .sum(),
        open_ms: values(|result| result.open_ms),
        prewarm_ms: values(|result| result.prewarm_ms),
        first_query_ms: values(|result| result.first_query_ms),
        total_ms: values(|result| result.total_ms),
        memory: ThreeWayColdMemorySummary {
            open_rss_bytes: memory(|result| &result.before, |result| &result.after_open),
            prewarm_rss_bytes: memory(|result| &result.after_open, |result| &result.after_prewarm),
            first_query_rss_bytes: memory(
                |result| &result.after_prewarm,
                |result| &result.after_first_query,
            ),
            total_rss_bytes: memory(|result| &result.before, |result| &result.after_first_query),
        },
        first_error: results.iter().find_map(|result| result.error.clone()),
    }
}

fn rss_delta(before: &MemorySnapshot, after: &MemorySnapshot) -> Option<i64> {
    Some(after.rss_bytes? as i64 - before.rss_bytes? as i64)
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn render_markdown(report: &ThreeWayColdBenchmarkReport) -> String {
    let mut markdown = String::from("# Core vs Proto vs SQLite Cold Start Benchmark Report\n\n");
    markdown.push_str(&format!("- Mode: `{}`\n", report.mode));
    markdown.push_str(&format!(
        "- Operation: `{}`\n- Query: `{}` / {}\n\n",
        report.operation, report.dimension, report.query
    ));
    let rows = [
        ("Core", &report.core),
        ("Proto", &report.proto),
        ("SQLite", &report.sqlite),
    ]
    .into_iter()
    .map(|(name, engine)| {
        vec![
            name.to_owned(),
            format_ms(engine.open_ms.p50_ms),
            format_ms(engine.prewarm_ms.p50_ms),
            format_ms(engine.first_query_ms.p50_ms),
            format_ms(engine.total_ms.p50_ms),
            format_signed_bytes(engine.memory.total_rss_bytes.p50_ms),
            engine.error_count.to_string(),
        ]
    })
    .collect::<Vec<_>>();
    markdown.push_str("## P50 Results\n\n");
    markdown.push_str(&markdown_table(
        &[
            "engine",
            "open",
            "prewarm",
            "first query",
            "total",
            "total RSS delta",
            "errors",
        ],
        &rows,
    ));
    markdown.push_str("\n## Notes\n\n");
    for note in &report.notes {
        markdown.push_str(&format!("- {note}\n"));
    }
    markdown
}

fn format_signed_bytes(value: f64) -> String {
    if value < 0.0 {
        format!("-{}", format_binary_bytes(value.abs() as u64))
    } else {
        format_binary_bytes(value as u64)
    }
}
