use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use range_store_core::bin_reader::BinReader;
use range_store_core::crc32c::crc32c;
use range_store_core::idx_reader::IdxReader;
use range_store_core::pack_codec::decode_pack;
use range_store_core::types::IdxRecord;

use crate::errors::ToolError;
use crate::metadata::load_action_schemas;
use crate::verification::float32_precision::{
    check_float32_round_trip, check_nullable_float32_round_trip, Float32CheckReason,
    Float32PrecisionStatsAccumulator, NullableFloat32CheckReason,
};
use crate::verification::report::{
    write_json_report, write_markdown_report, DimensionVerifyDetail, RangeStrataVerifyReport,
    VerifyFailure, VerifyLayer, VerifyMode, VerifyOptionsSummary, VerifyPrecision,
};
use crate::verification::standalone::{run_standalone_verify, StandaloneVerifyOptions};
use range_store_core::action_schema::ActionDef;
use range_store_core::dimension::{discover_dimensions, DimensionSpec};
use range_store_core::dimension::{get_bin_file_name, get_idx_file_name, quote_identifier};
use range_store_core::hole_cards::get_hand_id;
use range_store_core::manifest::{load_manifest, BuildManifest};
use range_store_core::sqlite::{Connection, Value};

const ACTION_VALUE_TOLERANCE: f64 = 1e-6;

#[derive(Debug, Clone)]
pub struct CrossVerifyOptions {
    pub dir: PathBuf,
    pub source_db: PathBuf,
    pub sample_size: usize,
    pub max_failures: usize,
    pub verify_checksums: bool,
    pub out_path: Option<PathBuf>,
    pub md_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct SourceRow {
    concrete_line_id: u32,
    hole_cards: String,
    action_name: String,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Option<f64>,
}

#[derive(Default)]
struct SourceCrossResult {
    failures: Vec<VerifyFailure>,
    checked_records: u64,
    failed_records: u64,
    extra_binary_records: u64,
    frequency_precision: Float32PrecisionStatsAccumulator,
    hand_ev_precision: Float32PrecisionStatsAccumulator,
    dimension_counts: HashMap<String, (u64, u64)>,
}

pub fn run_cross_verify(
    options: &CrossVerifyOptions,
) -> Result<RangeStrataVerifyReport, ToolError> {
    let standalone = run_standalone_verify(&StandaloneVerifyOptions {
        dir: options.dir.clone(),
        verify_checksums: options.verify_checksums,
        out_path: None,
        md_path: None,
    })?;
    let manifest = match load_manifest(&options.dir.join("manifest.json")) {
        Ok(manifest) => manifest,
        Err(_) => {
            let mut report = standalone;
            report.mode = VerifyMode::Cross;
            report.source_db_path = Some(options.source_db.display().to_string());
            write_cross_reports(&report, options)?;
            return Ok(report);
        }
    };

    let cross = run_source_cross(options, &manifest)?;
    let mut failures = standalone.failures.clone();
    failures.extend(cross.failures.clone());
    let dimensions = merge_dimension_counts(standalone.dimensions.clone(), &cross.dimension_counts);
    let report = RangeStrataVerifyReport::new(
        VerifyMode::Cross,
        options.dir.display().to_string(),
        Some(options.source_db.display().to_string()),
        options.verify_checksums,
        VerifyOptionsSummary {
            sample_size: Some(options.sample_size),
            max_failures: Some(options.max_failures),
        },
        dimensions,
        failures,
    )
    .with_cross_totals(
        cross.checked_records,
        cross.failed_records,
        cross.extra_binary_records,
        VerifyPrecision {
            frequency: cross.frequency_precision.to_summary(),
            hand_ev: cross.hand_ev_precision.to_summary(),
        },
    );
    write_cross_reports(&report, options)?;
    Ok(report)
}

fn run_source_cross(
    options: &CrossVerifyOptions,
    manifest: &BuildManifest,
) -> Result<SourceCrossResult, ToolError> {
    let mut result = SourceCrossResult::default();
    let source = Connection::open(&options.source_db, true)?;
    let dimensions = discover_dimensions(&source)?;
    let row_counts = dimension_row_counts(&source, &dimensions)?;
    let total_rows = row_counts.values().sum::<u64>().max(1);
    let action_schemas = load_action_schemas(&options.dir.join("meta.db"))?;

    for dimension in dimensions {
        if manifest_dimension_failed(manifest, &dimension) {
            continue;
        }
        let key = dimension_key(&dimension);
        let quota = if options.sample_size == 0 {
            None
        } else {
            let count = row_counts.get(&key).copied().unwrap_or_default();
            Some(
                ((count as f64 / total_rows as f64) * options.sample_size as f64)
                    .floor()
                    .max(1.0) as usize,
            )
        };
        let rows = load_source_rows(&source, &dimension, quota)?;
        verify_dimension_rows(options, &dimension, rows, &action_schemas, &mut result);
    }

    Ok(result)
}

fn verify_dimension_rows(
    options: &CrossVerifyOptions,
    dimension: &DimensionSpec,
    rows: Vec<SourceRow>,
    action_schemas: &HashMap<u32, Vec<ActionDef>>,
    result: &mut SourceCrossResult,
) {
    let key = dimension_key(dimension);
    let check = format!("dimension:{key}");
    let idx_path = options.dir.join(get_idx_file_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    ));
    let bin_path = options.dir.join(get_bin_file_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    ));
    let idx = match IdxReader::open(&idx_path) {
        Ok(idx) => idx,
        Err(error) => {
            push_failure(
                &mut result.failures,
                options.max_failures,
                VerifyFailure {
                    layer: VerifyLayer::SourceCross,
                    check,
                    reason: "IO_ERROR".to_owned(),
                    message: format!("Cannot read .idx file for {key}: {error}"),
                    context: None,
                },
            );
            return;
        }
    };
    let bin = match BinReader::open(&bin_path) {
        Ok(bin) => bin,
        Err(error) => {
            push_failure(
                &mut result.failures,
                options.max_failures,
                VerifyFailure {
                    layer: VerifyLayer::SourceCross,
                    check,
                    reason: "IO_ERROR".to_owned(),
                    message: format!("Cannot open .bin file for {key}: {error}"),
                    context: None,
                },
            );
            return;
        }
    };
    let idx_records: HashMap<u32, IdxRecord> = idx
        .records()
        .map(|record| (record.concrete_line_id, record))
        .collect();
    let mut by_line: BTreeMap<u32, Vec<SourceRow>> = BTreeMap::new();
    for row in rows {
        by_line.entry(row.concrete_line_id).or_default().push(row);
    }

    let mut dimension_checked = 0u64;
    let mut dimension_failed = 0u64;
    for (concrete_line_id, old_rows) in by_line {
        let Some(record) = idx_records.get(&concrete_line_id).cloned() else {
            for row in old_rows {
                result.checked_records += 1;
                result.failed_records += 1;
                dimension_checked += 1;
                dimension_failed += 1;
                push_failure(
                    &mut result.failures,
                    options.max_failures,
                    VerifyFailure {
                        layer: VerifyLayer::SourceCross,
                        check: format!("dimension:{key}"),
                        reason: "PACK_NOT_FOUND_IN_IDX".to_owned(),
                        message: format!(
                            "concreteLineId={concrete_line_id} found in source DB but not in .idx"
                        ),
                        context: Some(format!("line={concrete_line_id} hole={}", row.hole_cards)),
                    },
                );
            }
            continue;
        };
        let Some(actions) = action_schemas.get(&record.action_schema_id) else {
            mark_group_failed(
                options,
                result,
                &key,
                concrete_line_id,
                old_rows.len() as u64,
                "ACTION_SCHEMA_NOT_FOUND",
                format!("Missing action schema {}", record.action_schema_id),
            );
            dimension_checked += old_rows.len() as u64;
            dimension_failed += old_rows.len() as u64;
            continue;
        };
        let pack = match bin.read_pack(record.offset, record.byte_length) {
            Ok(pack) => pack,
            Err(error) => {
                mark_group_failed(
                    options,
                    result,
                    &key,
                    concrete_line_id,
                    old_rows.len() as u64,
                    "PACK_READ_ERROR",
                    error.to_string(),
                );
                dimension_checked += old_rows.len() as u64;
                dimension_failed += old_rows.len() as u64;
                continue;
            }
        };
        if options.verify_checksums {
            let actual_crc = crc32c(pack);
            if actual_crc != record.checksum {
                mark_group_failed(
                    options,
                    result,
                    &key,
                    concrete_line_id,
                    old_rows.len() as u64,
                    "CHECKSUM_MISMATCH",
                    format!("concreteLineId={concrete_line_id}: CRC mismatch"),
                );
                dimension_checked += old_rows.len() as u64;
                dimension_failed += old_rows.len() as u64;
                continue;
            }
        }
        let decoded = match decode_pack(pack, record.hand_count, actions.len() as u16) {
            Ok(decoded) => decoded,
            Err(error) => {
                mark_group_failed(
                    options,
                    result,
                    &key,
                    concrete_line_id,
                    old_rows.len() as u64,
                    "PACK_DECODE_ERROR",
                    error,
                );
                dimension_checked += old_rows.len() as u64;
                dimension_failed += old_rows.len() as u64;
                continue;
            }
        };
        let hand_index_by_id: HashMap<u8, usize> = decoded
            .hand_ids
            .iter()
            .copied()
            .enumerate()
            .map(|(index, hand_id)| (hand_id, index))
            .collect();
        let mut expected_binary_cells = HashSet::new();

        for row in old_rows {
            result.checked_records += 1;
            dimension_checked += 1;
            let row_failed = verify_source_row(
                options,
                &key,
                concrete_line_id,
                row,
                actions,
                &decoded,
                &hand_index_by_id,
                &mut expected_binary_cells,
                result,
            );
            if row_failed {
                result.failed_records += 1;
                dimension_failed += 1;
            }
        }

        if options.sample_size == 0 {
            for cell in decoded.cells.iter().filter(|cell| cell.exists) {
                if !expected_binary_cells.contains(&(cell.hand_id, cell.action_id)) {
                    result.extra_binary_records += 1;
                }
            }
        }
    }

    result
        .dimension_counts
        .insert(key, (dimension_checked, dimension_failed));
}

#[allow(clippy::too_many_arguments)]
fn verify_source_row(
    options: &CrossVerifyOptions,
    key: &str,
    concrete_line_id: u32,
    row: SourceRow,
    actions: &[ActionDef],
    decoded: &range_store_core::DecodedPack,
    hand_index_by_id: &HashMap<u8, usize>,
    expected_binary_cells: &mut HashSet<(u8, u32)>,
    result: &mut SourceCrossResult,
) -> bool {
    let context = format!(
        "line={concrete_line_id} hole={} action={}",
        row.hole_cards, row.action_name
    );
    let hand_id = match get_hand_id(&row.hole_cards) {
        Ok(hand_id) => hand_id,
        Err(_) => {
            push_source_failure(
                options,
                result,
                key,
                "UNKNOWN_HAND",
                "Unknown hand",
                context,
            );
            return true;
        }
    };
    let Some(hand_index) = hand_index_by_id.get(&hand_id).copied() else {
        push_source_failure(
            options,
            result,
            key,
            "HAND_NOT_FOUND_IN_PACK",
            format!("Hand {} in source but not in pack", row.hole_cards),
            context,
        );
        return true;
    };
    let Some(action) = find_matching_action(actions, &row) else {
        push_source_failure(
            options,
            result,
            key,
            "ACTION_NOT_FOUND_IN_SCHEMA",
            format!(
                "Action {}/{}/{} not in schema",
                row.action_name, row.action_size, row.amount_bb
            ),
            context,
        );
        return true;
    };
    let cell_index = hand_index * actions.len() + action.action_id as usize;
    let Some(cell) = decoded.cells.get(cell_index) else {
        push_source_failure(
            options,
            result,
            key,
            "ACTION_CELL_NOT_SET",
            "Cell index outside decoded pack",
            context,
        );
        return true;
    };
    if !cell.exists {
        push_source_failure(
            options,
            result,
            key,
            "ACTION_CELL_NOT_SET",
            format!("Cell not set for {}/{}", row.hole_cards, row.action_name),
            context,
        );
        return true;
    }
    expected_binary_cells.insert((hand_id, action.action_id));

    let frequency_check = check_float32_round_trip(row.frequency, cell.frequency);
    result
        .frequency_precision
        .add(frequency_check.clone(), context.clone());
    let hand_ev_check = check_nullable_float32_round_trip(row.hand_ev, cell.hand_ev);
    if let Some(value) = hand_ev_check.value.clone() {
        result.hand_ev_precision.add(value, context.clone());
    } else if hand_ev_check.reason == NullableFloat32CheckReason::NullMatch {
        result.hand_ev_precision.add_null();
    }

    if !frequency_check.ok {
        let reason = match frequency_check.reason {
            Float32CheckReason::Float32ValueMismatch => "FREQUENCY_FLOAT32_MISMATCH",
            _ => "FREQUENCY_INVALID_NUMBER",
        };
        push_source_failure(
            options,
            result,
            key,
            reason,
            format!(
                "source={}, expectedFloat32={}, actual={}, expectedBits={}, actualBits={}",
                row.frequency,
                frequency_check.expected_value,
                cell.frequency,
                crate::verification::float32_precision::format_float32_bits(
                    frequency_check.expected_bits
                ),
                crate::verification::float32_precision::format_float32_bits(
                    frequency_check.actual_bits
                )
            ),
            context,
        );
        return true;
    }

    if !hand_ev_check.ok {
        let reason = match hand_ev_check.reason {
            NullableFloat32CheckReason::NullMismatch => "HAND_EV_NULL_MISMATCH",
            NullableFloat32CheckReason::Float32ValueMismatch => "HAND_EV_FLOAT32_MISMATCH",
            _ => "HAND_EV_INVALID_NUMBER",
        };
        push_source_failure(
            options,
            result,
            key,
            reason,
            format!("source={:?}, actual={:?}", row.hand_ev, cell.hand_ev),
            context,
        );
        return true;
    }

    false
}

fn mark_group_failed(
    options: &CrossVerifyOptions,
    result: &mut SourceCrossResult,
    key: &str,
    concrete_line_id: u32,
    count: u64,
    reason: &str,
    message: String,
) {
    result.checked_records += count;
    result.failed_records += count;
    push_failure(
        &mut result.failures,
        options.max_failures,
        VerifyFailure {
            layer: VerifyLayer::SourceCross,
            check: format!("dimension:{key}"),
            reason: reason.to_owned(),
            message,
            context: Some(format!("line={concrete_line_id}")),
        },
    );
}

fn push_source_failure(
    options: &CrossVerifyOptions,
    result: &mut SourceCrossResult,
    key: &str,
    reason: &str,
    message: impl Into<String>,
    context: String,
) {
    push_failure(
        &mut result.failures,
        options.max_failures,
        VerifyFailure {
            layer: VerifyLayer::SourceCross,
            check: format!("dimension:{key}"),
            reason: reason.to_owned(),
            message: message.into(),
            context: Some(context),
        },
    );
}

fn push_failure(failures: &mut Vec<VerifyFailure>, max_failures: usize, failure: VerifyFailure) {
    if failures.len() < max_failures {
        failures.push(failure);
    }
}

fn find_matching_action<'a>(actions: &'a [ActionDef], row: &SourceRow) -> Option<&'a ActionDef> {
    let action_name = normalize_action_name(&row.action_name);
    actions.iter().find(|action| {
        action.action_name.as_str() == action_name
            && (f64::from(action.action_size) - row.action_size).abs() <= ACTION_VALUE_TOLERANCE
            && (f64::from(action.amount_bb) - row.amount_bb).abs() <= ACTION_VALUE_TOLERANCE
    })
}

fn normalize_action_name(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect()
}

fn dimension_row_counts(
    connection: &Connection,
    dimensions: &[DimensionSpec],
) -> Result<HashMap<String, u64>, ToolError> {
    let mut counts = HashMap::new();
    for dimension in dimensions {
        let table = quote_identifier(&dimension.range_table())?;
        let mut statement = connection.prepare(&format!("SELECT COUNT(*) FROM {table}"))?;
        statement.start(&[])?;
        if statement.step_row()? {
            counts.insert(
                dimension_key(dimension),
                statement.column_i64(0).max(0) as u64,
            );
        }
    }
    Ok(counts)
}

fn load_source_rows(
    connection: &Connection,
    dimension: &DimensionSpec,
    quota: Option<usize>,
) -> Result<Vec<SourceRow>, ToolError> {
    let table = quote_identifier(&dimension.range_table())?;
    let sql = if quota.is_some() {
        format!(
            "SELECT concrete_line_id, hole_cards, action_name, action_size,
                    amount_bb, frequency, hand_ev
             FROM {table}
             ORDER BY ((concrete_line_id * 1103515245 + id * 12345) & 2147483647)
             LIMIT ?1"
        )
    } else {
        format!(
            "SELECT concrete_line_id, hole_cards, action_name, action_size,
                    amount_bb, frequency, hand_ev
             FROM {table}
             ORDER BY concrete_line_id, hole_cards, action_name"
        )
    };
    let mut statement = connection.prepare(&sql)?;
    match quota {
        Some(quota) => statement.start(&[Value::from(quota)])?,
        None => statement.start(&[])?,
    }
    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push(SourceRow {
            concrete_line_id: statement.column_u32(0)?,
            hole_cards: statement.column_text(1)?,
            action_name: statement.column_text(2)?,
            action_size: statement.column_f64(3),
            amount_bb: statement.column_f64(4),
            frequency: statement.column_f64(5),
            hand_ev: statement.column_optional_f64(6),
        });
    }
    Ok(rows)
}

fn merge_dimension_counts(
    mut dimensions: Vec<DimensionVerifyDetail>,
    counts: &HashMap<String, (u64, u64)>,
) -> Vec<DimensionVerifyDetail> {
    for dimension in &mut dimensions {
        let key = format!(
            "{}:{}:{}",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        );
        if let Some((checked, failed)) = counts.get(&key) {
            dimension.source_cross_records = Some(*checked);
            dimension.source_cross_failures = Some(*failed as usize);
        }
    }
    dimensions
}

fn manifest_dimension_failed(manifest: &BuildManifest, dimension: &DimensionSpec) -> bool {
    manifest
        .dimensions
        .iter()
        .find(|candidate| {
            candidate.strategy == dimension.strategy
                && candidate.player_count == dimension.player_count
                && candidate.depth_bb == dimension.depth_bb
        })
        .is_some_and(|candidate| candidate.status.as_deref() == Some("failed"))
}

fn dimension_key(dimension: &DimensionSpec) -> String {
    format!(
        "{}:{}:{}",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

fn write_cross_reports(
    report: &RangeStrataVerifyReport,
    options: &CrossVerifyOptions,
) -> Result<(), ToolError> {
    if let Some(path) = &options.out_path {
        write_json_report(report, path)?;
    }
    if let Some(path) = &options.md_path {
        write_markdown_report(report, path)?;
    }
    Ok(())
}
