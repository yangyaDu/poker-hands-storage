use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::benchmark::benchmark_models::{
    concrete_lines_table_name, dimension_matches_requested, normalize_batch_sizes,
    range_table_name, BatchBenchmarkItem, BatchBenchmarkRequest, BatchQueriesBySize,
    BenchmarkWorkload, HandBenchmarkItem, WorkloadMode, WorkloadOptions,
};
use crate::domain::dimension::{quote_identifier, DimensionRef};
use crate::errors::AppError;
use crate::storage::sqlite::{Connection, Value};

#[derive(Debug, Clone)]
struct SamplingStats {
    dimension: DimensionRef,
    range_table: String,
    concrete_table: String,
    row_count: u64,
    min_id: u32,
    max_id: u32,
    concrete_row_count: u64,
    concrete_min_id: u32,
    concrete_max_id: u32,
}

#[derive(Debug, Clone, Copy)]
struct SampledRangeRow {
    concrete_line_id: u32,
}

pub fn create_benchmark_workload(options: &WorkloadOptions) -> Result<BenchmarkWorkload, AppError> {
    let connection = Connection::open(&options.source_db_path, true)?;
    let dimensions = discover_range_dimensions(&connection)?;
    let dimensions = dimensions
        .into_iter()
        .filter(|dimension| dimension_matches_requested(dimension, &options.requested_dimensions))
        .collect::<Vec<_>>();
    if dimensions.is_empty() {
        return Err(AppError::invalid_argument(
            "No range dimensions matched the requested benchmark filters.",
        ));
    }

    let stats = dimensions
        .iter()
        .map(|dimension| load_sampling_stats(&connection, dimension))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|stats| stats.row_count > 0)
        .collect::<Vec<_>>();
    if stats.is_empty() {
        return Err(AppError::invalid_argument(
            "No range rows were available for benchmark sampling.",
        ));
    }

    let batch_sizes = normalize_batch_sizes(options.batch_size, &options.batch_sizes);
    let mut sampler = WorkloadSampler::new(&connection, stats, options.seed);
    let mut batch_queries_by_size: BatchQueriesBySize = Vec::with_capacity(batch_sizes.len());
    for size in &batch_sizes {
        let queries = match options.workload_mode {
            WorkloadMode::Random => {
                sampler.sample_batch_queries(options.batch_iterations, *size)?
            }
            WorkloadMode::AbstractLocal => {
                sampler.sample_abstract_local_batch_queries(options.batch_iterations, *size)?
            }
        };
        batch_queries_by_size.push((*size, queries));
    }

    let batch_queries = batch_queries_by_size
        .iter()
        .find(|(size, _)| *size == options.batch_size.max(1))
        .or_else(|| batch_queries_by_size.first())
        .map(|(_, queries)| queries.clone())
        .unwrap_or_default();

    let hand_queries = match options.workload_mode {
        WorkloadMode::Random => sampler.sample_hand_queries(options.hand_iterations)?,
        WorkloadMode::AbstractLocal => {
            sampler.sample_abstract_local_hand_queries(options.hand_iterations)?
        }
    };

    Ok(BenchmarkWorkload {
        seed: options.seed,
        mode: options.workload_mode,
        dimensions: sampler
            .stats
            .iter()
            .map(|stats| dimension_key(&stats.dimension))
            .collect(),
        hand_queries,
        batch_queries,
        batch_size: options.batch_size.max(1),
        batch_queries_by_size,
    })
}

pub fn read_workload_json(path: &Path) -> Result<BenchmarkWorkload, AppError> {
    let raw = fs::read_to_string(path)?;
    let parsed: RawBenchmarkWorkload =
        serde_json::from_str(&raw).map_err(|error| AppError::invalid_format(error.to_string()))?;

    let mut batch_queries_by_size = parsed.batch_queries_by_size.unwrap_or_default();
    let batch_queries = parsed.batch_queries.unwrap_or_default();
    let batch_size = parsed.batch_size.unwrap_or(20).max(1);

    if batch_queries_by_size.is_empty() && !batch_queries.is_empty() {
        batch_queries_by_size.push((batch_size, batch_queries.clone()));
    }

    let fallback_batch_queries = batch_queries_by_size
        .first()
        .map(|(_, queries)| queries.clone())
        .unwrap_or_default();

    Ok(BenchmarkWorkload {
        seed: parsed.seed.unwrap_or_default(),
        mode: parsed.mode.unwrap_or(WorkloadMode::Random),
        dimensions: parsed.dimensions.unwrap_or_default(),
        hand_queries: parsed.hand_queries.unwrap_or_default(),
        batch_queries: if batch_queries.is_empty() {
            fallback_batch_queries
        } else {
            batch_queries
        },
        batch_size,
        batch_queries_by_size,
    })
}

pub fn write_workload_json(path: &Path, workload: &BenchmarkWorkload) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(workload)
        .map_err(|error| AppError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub fn parse_range_table_dimension(table_name: &str) -> Option<DimensionRef> {
    let rest = table_name.strip_prefix("range_data_")?;
    let rest = rest.strip_suffix("BB")?;
    let (left, depth) = rest.rsplit_once('_')?;
    let (strategy, player_count) = left.rsplit_once('_')?;
    let player_count = player_count.strip_suffix("max")?;
    Some(DimensionRef::new(
        strategy,
        player_count.parse().ok()?,
        depth.parse().ok()?,
    ))
}

fn discover_range_dimensions(connection: &Connection) -> Result<Vec<DimensionRef>, AppError> {
    let mut statement = connection.prepare(
        "SELECT name
         FROM sqlite_schema
         WHERE type = 'table' AND name LIKE 'range_data_%'
         ORDER BY name",
    )?;
    statement.start(&[])?;
    let mut dimensions = Vec::new();
    while statement.step_row()? {
        let table_name = statement.column_text(0)?;
        if let Some(dimension) = parse_range_table_dimension(&table_name) {
            dimensions.push(dimension);
        }
    }
    Ok(dimensions)
}

fn load_sampling_stats(
    connection: &Connection,
    dimension: &DimensionRef,
) -> Result<SamplingStats, AppError> {
    let range_table = range_table_name(dimension);
    let concrete_table = concrete_lines_table_name(dimension);
    let range_counts = load_table_counts(connection, &range_table)?;
    let concrete_counts = load_table_counts(connection, &concrete_table)?;
    Ok(SamplingStats {
        dimension: dimension.clone(),
        range_table,
        concrete_table,
        row_count: range_counts.2,
        min_id: range_counts.0,
        max_id: range_counts.1,
        concrete_row_count: concrete_counts.2,
        concrete_min_id: concrete_counts.0,
        concrete_max_id: concrete_counts.1,
    })
}

fn load_table_counts(
    connection: &Connection,
    table_name: &str,
) -> Result<(u32, u32, u64), AppError> {
    let table = quote_identifier(table_name)?;
    let sql = format!("SELECT MIN(id), MAX(id), COUNT(*) FROM {table}");
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[])?;
    if !statement.step_row()? {
        return Ok((0, 0, 0));
    }
    Ok((
        u32::try_from(statement.column_i64(0)).unwrap_or_default(),
        u32::try_from(statement.column_i64(1)).unwrap_or_default(),
        u64::try_from(statement.column_i64(2)).unwrap_or_default(),
    ))
}

struct WorkloadSampler<'a> {
    connection: &'a Connection,
    stats: Vec<SamplingStats>,
    random: SeededRandom,
    total_rows: u64,
    strata_indices: HashMap<String, usize>,
}

impl<'a> WorkloadSampler<'a> {
    fn new(connection: &'a Connection, stats: Vec<SamplingStats>, seed: u64) -> Self {
        let total_rows = stats.iter().map(|stats| stats.row_count).sum();
        let strata_indices = stats
            .iter()
            .map(|stats| (dimension_key(&stats.dimension), 0))
            .collect();
        Self {
            connection,
            stats,
            random: SeededRandom::new(seed),
            total_rows,
            strata_indices,
        }
    }

    fn sample_hand_queries(&mut self, count: usize) -> Result<Vec<HandBenchmarkItem>, AppError> {
        let mut result = Vec::with_capacity(count);
        let mut seen = HashSet::new();
        let max_attempts = (count * 20).max(count + 100);

        for _ in 0..max_attempts {
            if result.len() >= count {
                break;
            }
            let item = self.sample_stratified_hand_query()?;
            let key = format!(
                "{}:{}:{}",
                dimension_key(&item.dimension()),
                item.concrete_line_id,
                item.hole_cards
            );
            if seen.insert(key) {
                result.push(item);
            }
        }

        while result.len() < count {
            result.push(self.sample_stratified_hand_query()?);
        }
        Ok(result)
    }

    fn sample_batch_queries(
        &mut self,
        count: usize,
        batch_size: usize,
    ) -> Result<Vec<BatchBenchmarkItem>, AppError> {
        let mut result = Vec::with_capacity(count);
        let safe_batch_size = batch_size.max(1);

        for _ in 0..count {
            let stats = self.pick_stats().clone();
            let mut requests = Vec::with_capacity(safe_batch_size);
            let mut seen_in_batch = HashSet::new();
            let max_retries = safe_batch_size * 3;

            for _ in 0..safe_batch_size {
                let mut item = self.sample_hand_query(&stats)?;
                let mut key = format!("{}:{}", item.concrete_line_id, item.hole_cards);

                for _ in 0..max_retries {
                    if !seen_in_batch.contains(&key) {
                        break;
                    }
                    item = self.sample_hand_query(&stats)?;
                    key = format!("{}:{}", item.concrete_line_id, item.hole_cards);
                }

                seen_in_batch.insert(key);
                requests.push(BatchBenchmarkRequest {
                    concrete_line_id: item.concrete_line_id,
                    hole_cards: item.hole_cards,
                });
            }

            result.push(BatchBenchmarkItem {
                strategy: stats.dimension.strategy,
                player_count: stats.dimension.player_count,
                depth_bb: stats.dimension.depth_bb,
                requests,
            });
        }

        Ok(result)
    }

    fn sample_abstract_local_hand_queries(
        &mut self,
        count: usize,
    ) -> Result<Vec<HandBenchmarkItem>, AppError> {
        (0..count)
            .map(|_| self.sample_abstract_local_hand_query())
            .collect()
    }

    fn sample_abstract_local_batch_queries(
        &mut self,
        count: usize,
        batch_size: usize,
    ) -> Result<Vec<BatchBenchmarkItem>, AppError> {
        let mut result = Vec::with_capacity(count);
        let safe_batch_size = batch_size.max(1);

        for _ in 0..count {
            let stats = self.pick_stats().clone();
            let concrete_ids = self.sample_concrete_ids_for_abstract(&stats)?;
            if concrete_ids.is_empty() {
                let mut fallback = self.sample_batch_queries(1, safe_batch_size)?;
                result.push(fallback.remove(0));
                continue;
            }

            let start = self.random.next_int(concrete_ids.len());
            let mut requests = Vec::with_capacity(safe_batch_size);
            for request_index in 0..safe_batch_size {
                let concrete_line_id = concrete_ids[(start + request_index) % concrete_ids.len()];
                let item = self.sample_hand_for_concrete_line(&stats, concrete_line_id)?;
                requests.push(BatchBenchmarkRequest {
                    concrete_line_id: item.concrete_line_id,
                    hole_cards: item.hole_cards,
                });
            }

            result.push(BatchBenchmarkItem {
                strategy: stats.dimension.strategy,
                player_count: stats.dimension.player_count,
                depth_bb: stats.dimension.depth_bb,
                requests,
            });
        }
        Ok(result)
    }

    fn sample_stratified_hand_query(&mut self) -> Result<HandBenchmarkItem, AppError> {
        let stats = self.pick_stats().clone();
        let dim_key = dimension_key(&stats.dimension);
        let stratum_index = *self.strata_indices.get(&dim_key).unwrap_or(&0);
        self.strata_indices.insert(dim_key, (stratum_index + 1) % 5);

        let range_size = stats.max_id.saturating_sub(stats.min_id).saturating_add(1);
        let stratum_start =
            stats.min_id + (((stratum_index as u64 * range_size as u64) / 5) as u32);
        let stratum_end_offset = (((stratum_index + 1) as u64 * range_size as u64) / 5) as u32;
        let stratum_end = if stratum_end_offset == 0 {
            stratum_start
        } else {
            stats.min_id + stratum_end_offset - 1
        };
        let stratum_size = stratum_end.saturating_sub(stratum_start).saturating_add(1);
        let random_id = stratum_start + self.random.next_int(stratum_size as usize) as u32;
        self.sample_hand_query_by_id(&stats, random_id)
    }

    fn sample_hand_query(&mut self, stats: &SamplingStats) -> Result<HandBenchmarkItem, AppError> {
        let range_size = stats.max_id.saturating_sub(stats.min_id).saturating_add(1);
        let random_id = stats.min_id + self.random.next_int(range_size as usize) as u32;
        self.sample_hand_query_by_id(stats, random_id)
    }

    fn sample_hand_query_by_id(
        &self,
        stats: &SamplingStats,
        random_id: u32,
    ) -> Result<HandBenchmarkItem, AppError> {
        let row = self.sample_range_row_by_id(stats, random_id)?;
        let hole_cards = self.sample_hole_cards_for_concrete_line(stats, row.concrete_line_id)?;
        Ok(HandBenchmarkItem {
            strategy: stats.dimension.strategy.clone(),
            player_count: stats.dimension.player_count,
            depth_bb: stats.dimension.depth_bb,
            concrete_line_id: row.concrete_line_id,
            hole_cards,
        })
    }

    fn sample_abstract_local_hand_query(&mut self) -> Result<HandBenchmarkItem, AppError> {
        let stats = self.pick_stats().clone();
        let concrete_ids = self.sample_concrete_ids_for_abstract(&stats)?;
        if concrete_ids.is_empty() {
            return self.sample_hand_query(&stats);
        }
        let concrete_line_id = concrete_ids[self.random.next_int(concrete_ids.len())];
        self.sample_hand_for_concrete_line(&stats, concrete_line_id)
    }

    fn pick_stats(&mut self) -> &SamplingStats {
        let mut target = self.random.next() * self.total_rows as f64;
        for stats in &self.stats {
            target -= stats.row_count as f64;
            if target <= 0.0 {
                return stats;
            }
        }
        self.stats.last().expect("sampling stats are non-empty")
    }

    fn sample_range_row_by_id(
        &self,
        stats: &SamplingStats,
        random_id: u32,
    ) -> Result<SampledRangeRow, AppError> {
        let table = quote_identifier(&stats.range_table)?;
        let sql = format!(
            "SELECT concrete_line_id
             FROM {table}
             WHERE id >= ?1
             ORDER BY id
             LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;
        statement.start(&[Value::from(random_id)])?;
        if statement.step_row()? {
            return Ok(SampledRangeRow {
                concrete_line_id: statement.column_u32(0)?,
            });
        }

        let sql = format!(
            "SELECT concrete_line_id
             FROM {table}
             ORDER BY id
             LIMIT 1"
        );
        let mut fallback = self.connection.prepare(&sql)?;
        fallback.start(&[])?;
        if fallback.step_row()? {
            return Ok(SampledRangeRow {
                concrete_line_id: fallback.column_u32(0)?,
            });
        }
        Err(AppError::invalid_format(format!(
            "Could not sample row from {}",
            stats.range_table
        )))
    }

    fn sample_concrete_ids_for_abstract(
        &mut self,
        stats: &SamplingStats,
    ) -> Result<Vec<u32>, AppError> {
        if stats.concrete_row_count == 0 {
            return Ok(Vec::new());
        }
        let abstract_line = self.sample_abstract_line(stats)?;
        let table = quote_identifier(&stats.concrete_table)?;
        let sql = format!(
            "SELECT id
             FROM {table}
             WHERE abstract_line = ?1
             ORDER BY id"
        );
        let mut statement = self.connection.prepare(&sql)?;
        statement.start(&[Value::from(abstract_line)])?;
        let mut ids = Vec::new();
        while statement.step_row()? {
            ids.push(statement.column_u32(0)?);
        }
        Ok(ids)
    }

    fn sample_abstract_line(&mut self, stats: &SamplingStats) -> Result<String, AppError> {
        let range_size = stats
            .concrete_max_id
            .saturating_sub(stats.concrete_min_id)
            .saturating_add(1);
        let random_id = stats.concrete_min_id + self.random.next_int(range_size as usize) as u32;
        let table = quote_identifier(&stats.concrete_table)?;
        let sql = format!(
            "SELECT abstract_line
             FROM {table}
             WHERE id >= ?1
             ORDER BY id
             LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;
        statement.start(&[Value::from(random_id)])?;
        if statement.step_row()? {
            return statement.column_text(0).map_err(AppError::from);
        }

        let sql = format!(
            "SELECT abstract_line
             FROM {table}
             ORDER BY id
             LIMIT 1"
        );
        let mut fallback = self.connection.prepare(&sql)?;
        fallback.start(&[])?;
        if fallback.step_row()? {
            return fallback.column_text(0).map_err(AppError::from);
        }
        Err(AppError::invalid_format(format!(
            "Could not sample abstract line from {}",
            stats.concrete_table
        )))
    }

    fn sample_hand_for_concrete_line(
        &self,
        stats: &SamplingStats,
        concrete_line_id: u32,
    ) -> Result<HandBenchmarkItem, AppError> {
        let hole_cards = self.sample_hole_cards_for_concrete_line(stats, concrete_line_id)?;
        Ok(HandBenchmarkItem {
            strategy: stats.dimension.strategy.clone(),
            player_count: stats.dimension.player_count,
            depth_bb: stats.dimension.depth_bb,
            concrete_line_id,
            hole_cards,
        })
    }

    fn sample_hole_cards_for_concrete_line(
        &self,
        stats: &SamplingStats,
        concrete_line_id: u32,
    ) -> Result<String, AppError> {
        let table = quote_identifier(&stats.range_table)?;
        let sql = format!(
            "SELECT hole_cards
             FROM {table}
             WHERE concrete_line_id = ?1
             ORDER BY id
             LIMIT 1"
        );
        let mut statement = self.connection.prepare(&sql)?;
        statement.start(&[Value::from(concrete_line_id)])?;
        if statement.step_row()? {
            return statement.column_text(0).map_err(AppError::from);
        }
        let row = self.sample_range_row_by_id(stats, stats.min_id)?;
        let sql = format!(
            "SELECT hole_cards
             FROM {table}
             WHERE concrete_line_id = ?1
             ORDER BY id
             LIMIT 1"
        );
        let mut fallback = self.connection.prepare(&sql)?;
        fallback.start(&[Value::from(row.concrete_line_id)])?;
        if fallback.step_row()? {
            return fallback.column_text(0).map_err(AppError::from);
        }
        Err(AppError::invalid_format(format!(
            "Could not sample hand from {}",
            stats.range_table
        )))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawBenchmarkWorkload {
    seed: Option<u64>,
    mode: Option<WorkloadMode>,
    dimensions: Option<Vec<String>>,
    hand_queries: Option<Vec<HandBenchmarkItem>>,
    batch_queries: Option<Vec<BatchBenchmarkItem>>,
    batch_size: Option<usize>,
    batch_queries_by_size: Option<BatchQueriesBySize>,
}

struct SeededRandom {
    state: u32,
}

impl SeededRandom {
    fn new(seed: u64) -> Self {
        Self { state: seed as u32 }
    }

    fn next(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b79f5);
        let mut value = self.state;
        value = (value ^ (value >> 15)).wrapping_mul(value | 1);
        value ^= value.wrapping_add((value ^ (value >> 7)).wrapping_mul(value | 61));
        let value = value ^ (value >> 14);
        f64::from(value) / 4_294_967_296.0
    }

    fn next_int(&mut self, max_exclusive: usize) -> usize {
        (self.next() * max_exclusive.max(1) as f64).floor() as usize
    }
}

fn dimension_key(dimension: &DimensionRef) -> String {
    crate::domain::dimension::dimension_key(dimension)
}
