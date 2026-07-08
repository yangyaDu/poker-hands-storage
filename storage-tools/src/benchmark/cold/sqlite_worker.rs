use std::path::Path;
use std::time::Instant;

use crate::benchmark::cold::types::{ColdWorkerOutput, ColdWorkerTimings};
use crate::benchmark::memory_snapshot::{get_memory_snapshot, MemorySnapshot};
use crate::errors::ToolError;
use range_store_core::dimension::DimensionRef;
use range_store_core::sqlite::Connection;

pub struct SqliteColdWorkerParams<'a> {
    pub source: &'a Path,
    pub strategy: &'a str,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_id: u32,
    pub hand: &'a str,
}

pub fn run_sqlite_cold_worker(params: &SqliteColdWorkerParams<'_>) -> ColdWorkerOutput {
    let worker_start = Instant::now();
    let mut timings = ColdWorkerTimings {
        service_open_ms: 0.0,
        dimension_prewarm_ms: 0.0,
        first_query_ms: 0.0,
        close_ms: 0.0,
        worker_total_ms: 0.0,
    };

    let result = run_inner(params, &mut timings);
    timings.worker_total_ms = worker_start.elapsed().as_secs_f64() * 1000.0;

    match result {
        Ok((result_count, memory_before, memory_after)) => ColdWorkerOutput {
            ok: true,
            store_open_and_first_query_ms: timings.service_open_ms + timings.first_query_ms,
            result_count,
            memory_before,
            memory_after,
            timings,
            error: None,
        },
        Err(error) => ColdWorkerOutput {
            ok: false,
            store_open_and_first_query_ms: timings.service_open_ms + timings.first_query_ms,
            result_count: 0,
            memory_before: empty_snapshot(),
            memory_after: empty_snapshot(),
            timings,
            error: Some(error.to_string()),
        },
    }
}

fn run_inner(
    params: &SqliteColdWorkerParams<'_>,
    timings: &mut ColdWorkerTimings,
) -> Result<(usize, MemorySnapshot, MemorySnapshot), ToolError> {
    let open_start = Instant::now();
    let connection = Connection::open(params.source, true)?;
    timings.service_open_ms = open_start.elapsed().as_secs_f64() * 1000.0;

    let memory_before = get_memory_snapshot();

    let dimension = DimensionRef::new(params.strategy, params.player_count, params.depth_bb);
    let query_start = Instant::now();
    let result_count = crate::benchmark::hot::sqlite_runner::sqlite_action_count(
        &connection,
        &dimension,
        params.concrete_line_id,
        params.hand,
    )?;
    timings.first_query_ms = query_start.elapsed().as_secs_f64() * 1000.0;

    let memory_after = get_memory_snapshot();

    let close_start = Instant::now();
    drop(connection);
    timings.close_ms = close_start.elapsed().as_secs_f64() * 1000.0;

    Ok((result_count, memory_before, memory_after))
}

fn empty_snapshot() -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: None,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some("SQLite cold worker failed before memory snapshot.".to_owned()),
    }
}
