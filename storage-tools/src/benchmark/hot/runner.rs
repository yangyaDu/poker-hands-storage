use crate::benchmark::hot::result_verifier::verify_benchmark_results;
use crate::benchmark::hot::types::BenchmarkCommand;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, BenchmarkMemoryReport};
use crate::benchmark::metrics::{build_totals, measure_benchmark_case, BenchmarkCaseResult};
use crate::benchmark::report::{
    build_benchmark_report, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::types::{
    drill_scenario_table_name, BatchBenchmarkItem, BenchmarkWorkload, DrillScenarioBenchmarkItem,
    HandBenchmarkItem, HandsByActionsBenchmarkItem, WorkloadOptions, WorkloadSource,
};
use crate::benchmark::workload::{
    create_benchmark_workload, drill_depth_column, read_workload_json, table_exists,
    write_workload_json,
};
use crate::errors::ToolError;
use range_store_core::dimension::{quote_identifier, DimensionRef};
use range_store_core::query::{parse_action_filters, StoreQueryService};
use range_store_core::sqlite::{Connection, Value};

pub fn run_hot_benchmark(command: &BenchmarkCommand) -> Result<BenchmarkRunReport, ToolError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_mode = workload.mode;
    let memory_before = get_memory_snapshot();
    let service = StoreQueryService::open_with_meta(
        command.dir.clone(),
        command.meta.clone(),
        100,
        command.verify_checksums,
    )?;
    let meta_connection = Connection::open(&command.meta, true)?;

    prewarm_workload_dimensions(&service, &workload)?;

    let mut cases = Vec::new();
    cases.push(measure_hand_case(
        &service,
        &workload.hand_queries,
        command.warmup_iterations,
    ));
    cases.push(measure_batch_case(
        &service,
        "batch-hand-strategy",
        "Run a batch of concrete_line_id + hand lookups through Range Strata Binary batch API.",
        &workload.batch_queries,
        command.warmup_iterations,
    ));
    for (size, queries) in &workload.batch_queries_by_size {
        cases.push(measure_batch_case(
            &service,
            &format!("batch-size-{size}"),
            &format!("Run {size} lookups per batch through Range Strata Binary batch API."),
            queries,
            command.warmup_iterations,
        ));
    }
    cases.push(measure_hands_by_actions_case(
        &service,
        &workload.hands_by_actions_queries,
        command.warmup_iterations,
    ));
    cases.push(measure_drill_scenarios_case(
        &meta_connection,
        &workload.drill_scenario_queries,
        command.warmup_iterations,
    ));

    let memory_after = get_memory_snapshot();
    let memory = BenchmarkMemoryReport::new(memory_before, memory_after);
    let totals = build_totals(&cases);

    let mut notes = vec![
        "Rust Range Strata Binary hot benchmark; cold-start phase accounting lives in benchmark-cold."
            .to_owned(),
        "Result counts are case-specific: action entries for strategy lookups, matching hands for hands-by-actions, and abstract lines for drill metadata."
            .to_owned(),
        "`hands-by-actions` decodes binary packs through range-store-core and counts matching hands."
            .to_owned(),
        "`drill-scenarios-metadata` reads runtime meta.db SQLite tables; it is metadata-path evidence, not .idx/.bin strategy-pack performance."
            .to_owned(),
        "`batch-hand-strategy` is the default --batch-size case; `batch-size-*` entries are the sweep cases and should be summarized separately."
            .to_owned(),
        "No hard performance threshold is applied; use reports for local comparison and regression observation."
            .to_owned(),
    ];

    let result_verification = if command.verify_results {
        let verification =
            verify_benchmark_results(&command.source, &service, &workload.hand_queries)?;
        notes.extend(verification.notes());
        Some(verification)
    } else {
        None
    };

    let report = build_benchmark_report(ReportInput {
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
            verify_results: command.verify_results,
            workload_mode,
        },
        workload,
        workload_source,
        workload_path: benchmark_workload_path(command),
        cases,
        totals,
        memory,
        result_verification,
        notes,
    });

    write_benchmark_json(&command.out_path, &report)?;
    write_benchmark_markdown(&command.md_path, &report)?;

    Ok(report)
}

fn load_or_create_workload(
    command: &BenchmarkCommand,
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

fn benchmark_workload_path(command: &BenchmarkCommand) -> Option<String> {
    command
        .workload_path
        .as_ref()
        .or(command.write_workload_path.as_ref())
        .map(|path| path.display().to_string())
}

fn prewarm_workload_dimensions(
    service: &StoreQueryService,
    workload: &BenchmarkWorkload,
) -> Result<(), ToolError> {
    for dimension in &workload.dimensions {
        service.prewarm(&parse_workload_dimension(dimension)?)?;
    }
    Ok(())
}

fn measure_hand_case(
    service: &StoreQueryService,
    hand_queries: &[HandBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "hand-strategy",
        "Single concrete_line_id + hand query through Range Strata Binary StoreQueryService.",
        hand_queries,
        warmup_iterations,
        |item, _| query_hand_count(service, item),
    )
}

fn measure_batch_case(
    service: &StoreQueryService,
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
        |item, _| query_batch_count(service, item),
    )
}

fn measure_hands_by_actions_case(
    service: &StoreQueryService,
    queries: &[HandsByActionsBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "hands-by-actions",
        "Decode all hands for one concrete line and count hands matching any requested action_name.",
        queries,
        warmup_iterations,
        |item, _| query_hands_by_actions_count(service, item),
    )
}

fn measure_drill_scenarios_case(
    connection: &Connection,
    queries: &[DrillScenarioBenchmarkItem],
    warmup_iterations: usize,
) -> BenchmarkCaseResult {
    measure_benchmark_case(
        "drill-scenarios-metadata",
        "Read drill scenario abstract lines from runtime meta.db SQLite metadata tables.",
        queries,
        warmup_iterations,
        |item, _| drill_scenario_line_count(connection, item).map_err(|error| error.to_string()),
    )
}

fn query_hand_count(
    service: &StoreQueryService,
    item: &HandBenchmarkItem,
) -> Result<usize, String> {
    service
        .query(&item.dimension(), item.concrete_line_id, &item.hole_cards)
        .map(|result| result.actions.len())
        .map_err(|error| error.to_string())
}

fn query_batch_count(
    service: &StoreQueryService,
    item: &BatchBenchmarkItem,
) -> Result<usize, String> {
    let requests = item
        .requests
        .iter()
        .map(|request| (request.concrete_line_id, request.hole_cards.clone()))
        .collect::<Vec<_>>();
    let results = service
        .query_batch(&item.dimension(), &requests)
        .map_err(|error| error.to_string())?;
    let mut total = 0;
    for result in results.results {
        total += result.actions.len();
    }
    Ok(total)
}

fn query_hands_by_actions_count(
    service: &StoreQueryService,
    item: &HandsByActionsBenchmarkItem,
) -> Result<usize, String> {
    let action_filters =
        parse_action_filters(item.actions.clone()).map_err(|error| error.to_string())?;
    service
        .query_hands_by_actions(
            &item.dimension(),
            item.concrete_line_id,
            &action_filters,
            item.frequency,
        )
        .map(|hands| hands.len())
        .map_err(|error| error.to_string())
}

pub(crate) fn drill_scenario_line_count(
    connection: &Connection,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    let raw_table = drill_scenario_table_name(&item.strategy);
    if !table_exists(connection, &raw_table)? {
        return Err(ToolError::invalid_format(format!(
            "Drill scenario table not found: {raw_table}"
        )));
    }
    let depth_column = drill_depth_column(connection, &raw_table)?.ok_or_else(|| {
        ToolError::invalid_format(format!(
            "Drill scenario table {raw_table} must contain depth or drill_depth"
        ))
    })?;
    let table = quote_identifier(&raw_table)?;
    let sql = format!(
        "SELECT COUNT(DISTINCT abstract_line)
         FROM {table}
         WHERE drill_name = ?1 AND player_count = ?2 AND {depth_column} = ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[
        Value::from(item.drill_name.as_str()),
        Value::from(item.player_count),
        Value::from(item.drill_depth),
    ])?;
    if statement.step_row()? {
        Ok(usize::try_from(statement.column_i64(0)).unwrap_or_default())
    } else {
        Ok(0)
    }
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
