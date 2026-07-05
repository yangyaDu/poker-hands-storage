use std::collections::HashMap;

use crate::benchmark::hot::runner::drill_scenario_line_count;
use crate::benchmark::hot::types::BenchmarkCommand;
use crate::benchmark::memory_snapshot::{get_memory_snapshot, BenchmarkMemoryReport};
use crate::benchmark::metrics::{build_totals, measure_benchmark_case};
use crate::benchmark::report::{
    build_benchmark_report_for_engine, write_benchmark_json, write_benchmark_markdown,
    BenchmarkOptionsSummary, BenchmarkRunReport, ReportInput,
};
use crate::benchmark::types::{
    drill_scenario_table_name, BenchmarkWorkload, DrillScenarioBenchmarkItem, WorkloadOptions,
    WorkloadSource,
};
use crate::benchmark::workload::{
    create_benchmark_workload, drill_depth_column, read_workload_json, table_exists,
    write_workload_json,
};
use crate::errors::ToolError;
use range_store_core::dimension::quote_identifier;
use range_store_core::metadata::CachedMetadataReader;
use range_store_core::sqlite::{Connection, Statement, Value};

pub fn run_drill_metadata_benchmark(
    command: &BenchmarkCommand,
) -> Result<BenchmarkRunReport, ToolError> {
    let (workload, workload_source) = load_or_create_workload(command)?;
    let workload_mode = workload.mode;
    if workload.drill_scenario_queries.is_empty() {
        return Err(ToolError::invalid_argument(
            "No drill scenario queries were available for benchmark sampling.",
        ));
    }

    let memory_before = get_memory_snapshot();
    let connection = Connection::open(&command.meta, true)?;
    let mut prepared_statements =
        build_prepared_drill_statements(&connection, &workload.drill_scenario_queries)?;
    let cached_metadata =
        CachedMetadataReader::load(&command.dir, &command.meta).map_err(metadata_error)?;
    prefill_drill_cache(&cached_metadata, &workload.drill_scenario_queries)?;

    let cases = vec![
        measure_benchmark_case(
            "drill-raw-sqlite-schema-detect",
            "Read drill scenario abstract line count through the current raw SQLite path: schema detect, prepare SQL, and execute every iteration.",
            &workload.drill_scenario_queries,
            command.warmup_iterations,
            |item, _| drill_scenario_line_count(&connection, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            "drill-prepared-sqlite",
            "Read drill scenario abstract line count through prepared SQLite statements reused per strategy.",
            &workload.drill_scenario_queries,
            command.warmup_iterations,
            |item, _| prepared_drill_line_count(&mut prepared_statements, item).map_err(|error| error.to_string()),
        ),
        measure_benchmark_case(
            "drill-cached-metadata",
            "Read drill scenario abstract lines through CachedMetadataReader after cache prefill.",
            &workload.drill_scenario_queries,
            command.warmup_iterations,
            |item, _| cached_drill_line_count(&cached_metadata, item).map_err(|error| error.to_string()),
        ),
    ];

    let memory_after = get_memory_snapshot();
    let totals = build_totals(&cases);
    let report = build_benchmark_report_for_engine(
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
            memory: BenchmarkMemoryReport::new(memory_before, memory_after),
            result_verification: None,
            notes: vec![
                "This report isolates the drill metadata path only; it does not read range .idx/.bin strategy packs.".to_owned(),
                "`drill-raw-sqlite-schema-detect` intentionally preserves the old benchmark behavior that probes schema and prepares SQL on every iteration.".to_owned(),
                "`drill-prepared-sqlite` reuses one prepared COUNT(DISTINCT abstract_line) statement per strategy.".to_owned(),
                "`drill-cached-metadata` preloads the sampled drill keys into CachedMetadataReader and measures cache hits.".to_owned(),
            ],
        },
        "drill-metadata",
    );

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

fn build_prepared_drill_statements<'a>(
    connection: &'a Connection,
    queries: &[DrillScenarioBenchmarkItem],
) -> Result<HashMap<String, Statement<'a>>, ToolError> {
    let mut statements = HashMap::new();
    for item in queries {
        if statements.contains_key(&item.strategy) {
            continue;
        }
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
        statements.insert(item.strategy.clone(), connection.prepare(&sql)?);
    }
    Ok(statements)
}

fn prepared_drill_line_count(
    statements: &mut HashMap<String, Statement<'_>>,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    let statement = statements.get_mut(&item.strategy).ok_or_else(|| {
        ToolError::invalid_format(format!(
            "Prepared drill statement not found for strategy={}",
            item.strategy
        ))
    })?;
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

fn prefill_drill_cache(
    cached_metadata: &CachedMetadataReader,
    queries: &[DrillScenarioBenchmarkItem],
) -> Result<(), ToolError> {
    for item in queries {
        cached_metadata
            .get_drill_scenario_lines(
                &item.strategy,
                &item.drill_name,
                item.player_count,
                item.drill_depth,
            )
            .map_err(metadata_error)?;
    }
    Ok(())
}

fn cached_drill_line_count(
    cached_metadata: &CachedMetadataReader,
    item: &DrillScenarioBenchmarkItem,
) -> Result<usize, ToolError> {
    cached_metadata
        .get_drill_scenario_lines(
            &item.strategy,
            &item.drill_name,
            item.player_count,
            item.drill_depth,
        )
        .map(|lines| lines.len())
        .map_err(metadata_error)
}

fn metadata_error(error: range_store_core::metadata::MetadataError) -> ToolError {
    ToolError::new(error.code(), error.to_string())
}
