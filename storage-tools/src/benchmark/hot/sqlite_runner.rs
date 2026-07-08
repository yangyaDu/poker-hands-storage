use crate::benchmark::hot::runner::drill_scenario_line_count;
use crate::benchmark::hot::types::BenchmarkSqliteCommand;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, BenchmarkMemoryReport};
use crate::benchmark::metrics::{build_totals, measure_benchmark_case, BenchmarkCaseResult};
use crate::benchmark::report::{
    build_benchmark_report_for_engine, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::types::{
    concrete_lines_table_name, range_table_name, BatchBenchmarkItem, BenchmarkWorkload,
    ConcreteLineBenchmarkItem, DrillScenarioBenchmarkItem, HandBenchmarkItem,
    HandsByActionsBenchmarkItem, WorkloadOptions, WorkloadSource,
};
use crate::benchmark::workload::{
    build_concrete_line_lookup_queries, create_benchmark_workload, read_workload_json,
    ConcreteLineIdColumn,
};
use crate::errors::ToolError;
use range_store_core::dimension::quote_identifier;
use range_store_core::query::{
    parse_action_filter, ActionFilter, DEFAULT_HANDS_BY_ACTIONS_FREQUENCY,
};
use range_store_core::sqlite::{Connection, Value};

const ACTION_AMOUNT_TOLERANCE: f64 = 1e-6;

pub fn run_sqlite_benchmark(
    command: &BenchmarkSqliteCommand,
) -> Result<BenchmarkRunReport, ToolError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_mode = workload.mode;
    let connection = Connection::open(&command.source, true)?;
    let concrete_line_queries = build_concrete_line_lookup_queries(
        &connection,
        &workload.hand_queries,
        ConcreteLineIdColumn::SourceId,
    )?;

    let memory_before = get_memory_snapshot();
    let mut cases = Vec::new();
    cases.push(measure_concrete_lines_case(
        &connection,
        &concrete_line_queries,
        command.warmup_iterations,
    ));
    cases.push(measure_hand_case(
        &connection,
        &workload.hand_queries,
        command.warmup_iterations,
    ));
    cases.push(measure_batch_case(
        &connection,
        "batch-hand-strategy",
        "Run a batch of concrete_line_id + hand lookups through SQLite source tables.",
        &workload.batch_queries,
        command.warmup_iterations,
    ));
    for (size, queries) in &workload.batch_queries_by_size {
        cases.push(measure_batch_case(
            &connection,
            &format!("batch-size-{size}"),
            &format!("Run {size} lookups per batch through SQLite source tables."),
            queries,
            command.warmup_iterations,
        ));
    }
    cases.push(measure_hands_by_actions_case(
        &connection,
        &workload.hands_by_actions_queries,
        command.warmup_iterations,
    ));
    cases.push(measure_drill_scenarios_case(
        &connection,
        &workload.drill_scenario_queries,
        command.warmup_iterations,
    ));

    let memory_after = get_memory_snapshot();
    let memory = BenchmarkMemoryReport::new(memory_before, memory_after);
    let totals = build_totals(&cases);

    let report = build_benchmark_report_for_engine(
        ReportInput {
            source_db_path: command.source.display().to_string(),
            binary_dir: "not-applicable".to_owned(),
            meta_db_path: "not-applicable".to_owned(),
            options: BenchmarkOptionsSummary {
                seed: command.seed,
                requested_dimensions: command.requested_dimension_values.clone(),
                hand_iterations: command.hand_iterations,
                batch_iterations: command.batch_iterations,
                batch_size: command.batch_size,
                batch_sizes: command.batch_sizes.clone(),
                warmup_iterations: command.warmup_iterations,
                verify_checksums: false,
                verify_results: false,
                workload_mode,
            },
            workload,
            workload_source,
            workload_path: command
                .workload_path
                .as_ref()
                .map(|path| path.display().to_string()),
            cases,
            totals,
            memory,
            result_verification: None,
            notes: vec![
                "SQLite baseline benchmark; queries read and count metadata rows and action rows from source SQLite tables.".to_owned(),
                "`concrete-lines-exact` resolves concrete_line through source concrete_lines_* tables; samples are derived from hand workload and skip empty concrete_line rows.".to_owned(),
                "`hands-by-actions` uses source SQLite DISTINCT hole_cards with OR action-filter semantics and strict frequency threshold.".to_owned(),
                "`drill-scenarios-metadata` reads source SQLite drill_scenario_lines_* metadata tables; compare only against the runtime meta.db metadata path.".to_owned(),
                "`batch-hand-strategy` is the default --batch-size case; `batch-size-*` entries are the sweep cases and should be summarized separately.".to_owned(),
                "No hard performance threshold is applied; compare reports from the same workload before drawing conclusions.".to_owned(),
            ],
        },
        "sqlite",
    );

    write_benchmark_json(&command.out_path, &report)?;
    write_benchmark_markdown(&command.md_path, &report)?;
    Ok(report)
}

fn load_or_create_workload(
    command: &BenchmarkSqliteCommand,
) -> Result<(BenchmarkWorkload, WorkloadSource), ToolError> {
    if let Some(path) = &command.workload_path {
        Ok((read_workload_json(path)?, WorkloadSource::Loaded))
    } else {
        Ok((
            create_benchmark_workload(&WorkloadOptions {
                source_db_path: command.source.clone(),
                requested_dimensions: command.requested_dimensions.clone(),
                seed: command.seed,
                hand_iterations: command.hand_iterations,
                batch_iterations: command.batch_iterations,
                batch_size: command.batch_size,
                batch_sizes: command.batch_sizes.clone(),
                workload_mode: command.workload_mode,
            })?,
            WorkloadSource::Generated,
        ))
    }
}

fn measure_concrete_lines_case(
    connection: &Connection,
    queries: &[ConcreteLineBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "concrete-lines-exact",
        "Resolve concrete_line through source SQLite concrete_lines_* exact lookup.",
        queries,
        warmup_iterations,
        |item, _| sqlite_concrete_line_count(connection, item).map_err(|error| error.to_string()),
    )
}

fn measure_hand_case(
    connection: &Connection,
    hand_queries: &[HandBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "hand-strategy",
        "Single concrete_line_id + hand query through SQLite source tables.",
        hand_queries,
        warmup_iterations,
        |item, _| query_hand_count(connection, item),
    )
}

fn measure_batch_case(
    connection: &Connection,
    name: &str,
    description: &str,
    batch_queries: &[BatchBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        name,
        description,
        batch_queries,
        warmup_iterations,
        |item, _| query_batch_count(connection, item),
    )
}

fn measure_hands_by_actions_case(
    connection: &Connection,
    queries: &[HandsByActionsBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "hands-by-actions",
        "Count distinct source SQLite hole_cards matching any requested action filter.",
        queries,
        warmup_iterations,
        |item, _| {
            sqlite_hands_by_actions_count(connection, item).map_err(|error| error.to_string())
        },
    )
}

fn measure_drill_scenarios_case(
    connection: &Connection,
    queries: &[DrillScenarioBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "drill-scenarios-metadata",
        "Read drill scenario abstract lines from source SQLite metadata tables.",
        queries,
        warmup_iterations,
        |item, _| drill_scenario_line_count(connection, item).map_err(|error| error.to_string()),
    )
}

fn query_hand_count(connection: &Connection, item: &HandBenchmarkItem) -> Result<usize, String> {
    sqlite_action_count(
        connection,
        &item.dimension(),
        item.concrete_line_id,
        &item.hole_cards,
    )
    .map_err(|error| error.to_string())
}

fn query_batch_count(connection: &Connection, item: &BatchBenchmarkItem) -> Result<usize, String> {
    let mut total = 0;
    let dimension = item.dimension();
    for request in &item.requests {
        total += sqlite_action_count(
            connection,
            &dimension,
            request.concrete_line_id,
            &request.hole_cards,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(total)
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
    if ids.len() != 1 {
        return Err(ToolError::invalid_format(format!(
            "expected one concrete line, got {}",
            ids.len()
        )));
    }
    let concrete_line_id = ids[0];
    if concrete_line_id != item.concrete_line_id {
        return Err(ToolError::invalid_format(format!(
            "concrete line id mismatch: expected {}, got {}",
            item.concrete_line_id, concrete_line_id
        )));
    }
    Ok(ids.len())
}

fn sqlite_hands_by_actions_count(
    connection: &Connection,
    item: &HandsByActionsBenchmarkItem,
) -> Result<usize, ToolError> {
    let table = quote_identifier(&range_table_name(&item.dimension()))?;
    let threshold = item.frequency.unwrap_or(DEFAULT_HANDS_BY_ACTIONS_FREQUENCY);
    let mut values = vec![Value::from(item.concrete_line_id), Value::from(threshold)];
    let sql = if item.actions.is_empty() {
        format!(
            "SELECT COUNT(DISTINCT hole_cards)
             FROM {table}
             WHERE concrete_line_id = ?1 AND frequency > ?2"
        )
    } else {
        let filters = item
            .actions
            .iter()
            .map(|action| parse_action_filter(action))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ToolError::invalid_argument(error.to_string()))?;
        sqlite_hands_by_actions_or_sql(&table, &filters, &mut values)
    };
    let mut statement = connection.prepare(&sql)?;
    statement.start(&values)?;
    if statement.step_row()? {
        Ok(usize::try_from(statement.column_i64(0)).unwrap_or_default())
    } else {
        Ok(0)
    }
}

fn sqlite_hands_by_actions_or_sql(
    table: &str,
    filters: &[ActionFilter],
    values: &mut Vec<Value>,
) -> String {
    let mut action_clauses = Vec::with_capacity(filters.len());
    for filter in filters {
        let action_param = values.len() + 1;
        values.push(Value::from(filter.action_name.as_str()));

        let amount_clause = if let Some(amount_bb) = filter.amount_bb {
            let amount_param = values.len() + 1;
            values.push(Value::from(f64::from(amount_bb)));
            format!(" AND ABS(amount_bb - ?{amount_param}) <= {ACTION_AMOUNT_TOLERANCE}")
        } else {
            String::new()
        };

        action_clauses.push(format!("(action_name = ?{action_param}{amount_clause})"));
    }

    format!(
        "SELECT COUNT(DISTINCT hole_cards)
         FROM {table}
         WHERE concrete_line_id = ?1
           AND frequency > ?2
           AND ({})",
        action_clauses.join(" OR ")
    )
}

pub(crate) fn sqlite_action_count(
    connection: &Connection,
    dimension: &range_store_core::dimension::DimensionRef,
    concrete_line_id: u32,
    hole_cards: &str,
) -> Result<usize, ToolError> {
    let table = quote_identifier(&range_table_name(dimension))?;
    let sql = format!(
        "SELECT action_name, action_size, amount_bb, frequency, hand_ev
         FROM {table}
         WHERE concrete_line_id = ?1 AND hole_cards = ?2
         ORDER BY id"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[Value::from(concrete_line_id), Value::from(hole_cards)])?;
    let mut count = 0;
    while statement.step_row()? {
        let _action_name = statement.column_text(0)?;
        let _action_size = statement.column_f64(1);
        let _amount_bb = statement.column_f64(2);
        let _frequency = statement.column_f64(3);
        let _hand_ev = statement.column_optional_f64(4);
        count += 1;
    }
    Ok(count)
}
