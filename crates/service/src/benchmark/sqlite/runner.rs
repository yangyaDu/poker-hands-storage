use crate::benchmark::memory_snapshot::{get_memory_snapshot, BenchmarkMemoryReport};
use crate::benchmark::metrics::{build_totals, measure_benchmark_case, BenchmarkCaseResult};
use crate::benchmark::report::{
    build_benchmark_report_for_engine, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::sqlite::types::BenchmarkSqliteCommand;
use crate::benchmark::types::{
    range_table_name, BatchBenchmarkItem, BenchmarkWorkload, HandBenchmarkItem, WorkloadOptions,
    WorkloadSource,
};
use crate::benchmark::workload::{create_benchmark_workload, read_workload_json};
use crate::domain::dimension::quote_identifier;
use crate::errors::AppError;
use crate::storage::sqlite::{Connection, Value};

pub fn run_sqlite_benchmark(
    command: &BenchmarkSqliteCommand,
) -> Result<BenchmarkRunReport, AppError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_mode = workload.mode;
    let connection = Connection::open(&command.source, true)?;

    let memory_before = get_memory_snapshot();
    let mut cases = Vec::new();
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
                "SQLite baseline benchmark; queries read and count action rows from source range_data tables.".to_owned(),
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
) -> Result<(BenchmarkWorkload, WorkloadSource), AppError> {
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

pub(crate) fn sqlite_action_count(
    connection: &Connection,
    dimension: &crate::domain::dimension::DimensionRef,
    concrete_line_id: u32,
    hole_cards: &str,
) -> Result<usize, AppError> {
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
