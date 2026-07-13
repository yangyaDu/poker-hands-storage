use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use range_store_core::crc32c::crc32c;
use range_store_core::types::{
    IDX_HEADER_SIZE, IDX_MAGIC, IDX_RECORD_SIZE, PFSP_HEADER_SIZE, PFSP_MAGIC,
};

use crate::errors::ToolError;
use crate::verification::catalog_checks::check_catalog;
use crate::verification::report::{
    write_json_report, write_markdown_report, DimensionVerifyDetail, RangeStrataVerifyReport,
    VerifyFailure, VerifyLayer, VerifyMode, VerifyOptionsSummary,
};
use range_store_core::dimension::{get_concrete_lines_table_name, quote_identifier};
use range_store_core::manifest::{parse_manifest, BuildManifest, ManifestDimension, ManifestError};
use range_store_core::sqlite::Connection;

#[derive(Debug, Clone)]
pub struct StandaloneVerifyOptions {
    pub dir: PathBuf,
    pub verify_checksums: bool,
    pub out_path: Option<PathBuf>,
    pub md_path: Option<PathBuf>,
}

pub fn run_standalone_verify(
    options: &StandaloneVerifyOptions,
) -> Result<RangeStrataVerifyReport, ToolError> {
    let mut failures = Vec::new();
    let manifest = match read_manifest(&options.dir) {
        Ok(manifest) => manifest,
        Err(failure) => {
            failures.push(failure);
            let report = RangeStrataVerifyReport::new(
                VerifyMode::Standalone,
                options.dir.display().to_string(),
                None,
                options.verify_checksums,
                VerifyOptionsSummary::default(),
                Vec::new(),
                failures,
            );
            write_reports(&report, options)?;
            return Ok(report);
        }
    };

    check_files_exist(&options.dir, &manifest, &mut failures);
    let (catalog, catalog_failures) = check_catalog(&options.dir, &manifest);
    failures.extend(catalog_failures);
    let index_record_counts = check_index_headers(
        &options.dir,
        &manifest,
        &catalog.valid_action_schema_ids,
        &mut failures,
    );
    check_implicit_line_id_layout(&options.dir, &manifest, &index_record_counts, &mut failures);
    check_pack_headers(&options.dir, &manifest, &mut failures);
    check_index_pack_cross(
        &options.dir,
        &manifest,
        &catalog.action_counts,
        options,
        &mut failures,
    );

    let dimensions =
        build_dimension_details(&options.dir, &manifest, &index_record_counts, &failures);
    let report = RangeStrataVerifyReport::new(
        VerifyMode::Standalone,
        options.dir.display().to_string(),
        None,
        options.verify_checksums,
        VerifyOptionsSummary::default(),
        dimensions,
        failures,
    );
    write_reports(&report, options)?;
    Ok(report)
}

fn read_manifest(dir: &std::path::Path) -> Result<BuildManifest, VerifyFailure> {
    let path = dir.join("manifest.json");
    let raw = fs::read_to_string(&path).map_err(|error| VerifyFailure {
        layer: VerifyLayer::FileExistence,
        check: "manifest.json".to_owned(),
        reason: "MISSING_FILE".to_owned(),
        message: format!("manifest.json not found in {}: {error}", dir.display()),
        context: None,
    })?;

    parse_manifest(&raw).map_err(|error| VerifyFailure {
        layer: VerifyLayer::Manifest,
        check: "schema".to_owned(),
        reason: manifest_reason(&error).to_owned(),
        message: error.to_string(),
        context: None,
    })
}

fn manifest_reason(error: &ManifestError) -> &'static str {
    match error {
        ManifestError::Json(_) => "INVALID_JSON",
        ManifestError::UnsupportedFormat { .. } => "UNSUPPORTED_FORMAT",
        ManifestError::MissingDimensionFile { .. } => "MISSING_FILE",
        ManifestError::Io(_) => "IO_ERROR",
    }
}

fn check_files_exist(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    failures: &mut Vec<VerifyFailure>,
) {
    if !dir.join("meta.db").is_file() {
        failures.push(VerifyFailure {
            layer: VerifyLayer::FileExistence,
            check: "meta.db".to_owned(),
            reason: "MISSING_FILE".to_owned(),
            message: format!("meta.db not found at {}", dir.join("meta.db").display()),
            context: None,
        });
    }

    for dimension in successful_dimensions(manifest) {
        let key = dimension_key(dimension);
        match &dimension.idx_file {
            Some(file) if !dir.join(file).is_file() => failures.push(VerifyFailure {
                layer: VerifyLayer::FileExistence,
                check: format!("dimension:{key}"),
                reason: "MISSING_FILE".to_owned(),
                message: format!(".idx file not found: {}", dir.join(file).display()),
                context: None,
            }),
            None => failures.push(VerifyFailure {
                layer: VerifyLayer::FileExistence,
                check: format!("dimension:{key}"),
                reason: "MISSING_FILE".to_owned(),
                message: format!("manifest dimension {key} has no idxFile"),
                context: None,
            }),
            _ => {}
        }
        match &dimension.bin_file {
            Some(file) if !dir.join(file).is_file() => failures.push(VerifyFailure {
                layer: VerifyLayer::FileExistence,
                check: format!("dimension:{key}"),
                reason: "MISSING_FILE".to_owned(),
                message: format!(".bin file not found: {}", dir.join(file).display()),
                context: None,
            }),
            None => failures.push(VerifyFailure {
                layer: VerifyLayer::FileExistence,
                check: format!("dimension:{key}"),
                reason: "MISSING_FILE".to_owned(),
                message: format!("manifest dimension {key} has no binFile"),
                context: None,
            }),
            _ => {}
        }
    }
}

fn check_index_headers(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    valid_action_schema_ids: &std::collections::HashSet<u32>,
    failures: &mut Vec<VerifyFailure>,
) -> HashMap<String, u32> {
    let mut record_counts = HashMap::new();
    for dimension in successful_dimensions(manifest) {
        let key = dimension_key(dimension);
        let check = format!("dimension:{key}");
        let Some(idx_file) = &dimension.idx_file else {
            continue;
        };
        let path = dir.join(idx_file);
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(error) => {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::IndexHeader,
                    check,
                    reason: "IO_ERROR".to_owned(),
                    message: format!("Cannot read .idx file {}: {error}", path.display()),
                    context: None,
                });
                continue;
            }
        };
        if raw.len() < IDX_HEADER_SIZE {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexHeader,
                check,
                reason: "INVALID_FILE_SIZE".to_owned(),
                message: format!(
                    ".idx file {idx_file} is too small ({} bytes, min {IDX_HEADER_SIZE})",
                    raw.len()
                ),
                context: None,
            });
            continue;
        }

        if raw[0..4] != IDX_MAGIC[..] {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexHeader,
                check: check.clone(),
                reason: "INVALID_MAGIC".to_owned(),
                message: format!(".idx magic expected PFXI in {}", path.display()),
                context: None,
            });
        }
        let version = u16_from_le(&raw[4..6]);
        if version != 1 {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexHeader,
                check: check.clone(),
                reason: "UNSUPPORTED_VERSION".to_owned(),
                message: format!(".idx version expected 1, got {version}"),
                context: None,
            });
        }
        let record_count = u32_from_le(&raw[8..12]);
        record_counts.insert(key, record_count);
        let header_size = u16_from_le(&raw[12..14]);
        if header_size as usize != IDX_HEADER_SIZE {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexHeader,
                check: check.clone(),
                reason: "INVALID_HEADER_SIZE".to_owned(),
                message: format!(".idx headerSize expected {IDX_HEADER_SIZE}, got {header_size}"),
                context: None,
            });
        }
        let expected_len = IDX_HEADER_SIZE + record_count as usize * IDX_RECORD_SIZE;
        if raw.len() != expected_len {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexHeader,
                check,
                reason: "INVALID_FILE_SIZE".to_owned(),
                message: format!(".idx file size {} != expected {expected_len}", raw.len()),
                context: None,
            });
            continue;
        }

        for index in 0..record_count as usize {
            let concrete_line_id = index as u32 + 1;
            let offset = IDX_HEADER_SIZE + index * IDX_RECORD_SIZE;
            let record = decode_idx_record(&raw[offset..offset + IDX_RECORD_SIZE]);
            if record.hand_count > 169 {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::IndexHeader,
                    check: check.clone(),
                    reason: "INVALID_HAND_COUNT".to_owned(),
                    message: format!(
                        ".idx record concreteLineId={concrete_line_id}: handCount={} out of range [0, 169]",
                        record.hand_count
                    ),
                    context: None,
                });
            }
            if !valid_action_schema_ids.contains(&record.action_schema_id) {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::IndexHeader,
                    check: check.clone(),
                    reason: "DANGLING_FOREIGN_KEY".to_owned(),
                    message: format!(
                        ".idx record concreteLineId={concrete_line_id}: actionSchemaId={} not found in meta.db.action_schemas",
                        record.action_schema_id
                    ),
                    context: None,
                });
            }
        }
    }
    record_counts
}

fn check_implicit_line_id_layout(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    record_counts: &HashMap<String, u32>,
    failures: &mut Vec<VerifyFailure>,
) {
    let meta_path = dir.join("meta.db");
    let connection = match Connection::open(&meta_path, true) {
        Ok(connection) => connection,
        Err(error) => {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "meta.db".to_owned(),
                reason: "IO_ERROR".to_owned(),
                message: format!("Cannot open meta.db for implicit id validation: {error}"),
                context: None,
            });
            return;
        }
    };

    for dimension in successful_dimensions(manifest) {
        let key = dimension_key(dimension);
        let Some(&record_count) = record_counts.get(&key) else {
            continue;
        };
        let table_name = get_concrete_lines_table_name(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        );
        let table = match quote_identifier(&table_name) {
            Ok(table) => table,
            Err(error) => {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::Catalog,
                    check: format!("dimension:{key}"),
                    reason: "INVALID_TABLE_NAME".to_owned(),
                    message: error.to_string(),
                    context: None,
                });
                continue;
            }
        };
        let sql = format!("SELECT concrete_line_id FROM {table} ORDER BY concrete_line_id");
        let mut statement = match connection.prepare(&sql) {
            Ok(statement) => statement,
            Err(error) => {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::Catalog,
                    check: format!("dimension:{key}"),
                    reason: "IO_ERROR".to_owned(),
                    message: format!("Cannot read implicit concrete line ids: {error}"),
                    context: None,
                });
                continue;
            }
        };
        if let Err(error) = statement.start(&[]) {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: format!("dimension:{key}"),
                reason: "IO_ERROR".to_owned(),
                message: format!("Cannot iterate implicit concrete line ids: {error}"),
                context: None,
            });
            continue;
        }

        let mut observed_count = 0u32;
        while matches!(statement.step_row(), Ok(true)) {
            let expected_id = observed_count + 1;
            let actual_id = match statement.column_u32(0) {
                Ok(id) => id,
                Err(error) => {
                    failures.push(VerifyFailure {
                        layer: VerifyLayer::Catalog,
                        check: format!("dimension:{key}"),
                        reason: "INVALID_FORMAT".to_owned(),
                        message: error.to_string(),
                        context: None,
                    });
                    break;
                }
            };
            if actual_id != expected_id {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::Catalog,
                    check: format!("dimension:{key}"),
                    reason: "NON_DENSE_CONCRETE_LINE_ID".to_owned(),
                    message: format!(
                        "meta.db concrete line id at record {observed_count} is {actual_id}, expected {expected_id}"
                    ),
                    context: None,
                });
            }
            observed_count += 1;
        }
        if observed_count != record_count {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: format!("dimension:{key}"),
                reason: "CONCRETE_LINE_COUNT_MISMATCH".to_owned(),
                message: format!(
                    "meta.db has {observed_count} concrete lines but .idx declares {record_count} records"
                ),
                context: None,
            });
        }
    }
}
fn check_pack_headers(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    failures: &mut Vec<VerifyFailure>,
) {
    for dimension in successful_dimensions(manifest) {
        let key = dimension_key(dimension);
        let check = format!("dimension:{key}");
        let Some(bin_file) = &dimension.bin_file else {
            continue;
        };
        let path = dir.join(bin_file);
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(error) => {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::PackHeader,
                    check,
                    reason: "IO_ERROR".to_owned(),
                    message: format!("Cannot read .bin file {}: {error}", path.display()),
                    context: None,
                });
                continue;
            }
        };
        if raw.len() < PFSP_HEADER_SIZE {
            failures.push(VerifyFailure {
                layer: VerifyLayer::PackHeader,
                check,
                reason: "INVALID_FILE_SIZE".to_owned(),
                message: format!(
                    ".bin file {bin_file} is too small ({} bytes, min {PFSP_HEADER_SIZE})",
                    raw.len()
                ),
                context: None,
            });
            continue;
        }
        if let Some(message) = validate_bin_header(&raw[..PFSP_HEADER_SIZE]) {
            failures.push(VerifyFailure {
                layer: VerifyLayer::PackHeader,
                check,
                reason: "INVALID_HEADER".to_owned(),
                message: format!(".bin file {bin_file}: {message}"),
                context: None,
            });
        }
    }
}

fn check_index_pack_cross(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    action_counts: &HashMap<u32, u32>,
    options: &StandaloneVerifyOptions,
    failures: &mut Vec<VerifyFailure>,
) {
    for dimension in successful_dimensions(manifest) {
        let key = dimension_key(dimension);
        let check = format!("dimension:{key}");
        let (Some(idx_file), Some(bin_file)) = (&dimension.idx_file, &dimension.bin_file) else {
            continue;
        };
        let idx_raw = match fs::read(dir.join(idx_file)) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let bin_raw = match fs::read(dir.join(bin_file)) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if idx_raw.len() < IDX_HEADER_SIZE {
            continue;
        }
        let record_count = u32_from_le(&idx_raw[8..12]) as usize;
        let expected_idx_len = IDX_HEADER_SIZE + record_count * IDX_RECORD_SIZE;
        if idx_raw.len() != expected_idx_len {
            continue;
        }

        for index in 0..record_count {
            let record_offset = IDX_HEADER_SIZE + index * IDX_RECORD_SIZE;
            let record =
                decode_idx_record(&idx_raw[record_offset..record_offset + IDX_RECORD_SIZE]);
            let concrete_line_id = index as u32 + 1;
            if record.offset < PFSP_HEADER_SIZE as u32 {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::IndexPackCross,
                    check: check.clone(),
                    reason: "INVALID_OFFSET".to_owned(),
                    message: format!(
                        ".idx record concreteLineId={}: offset={} is within .bin header",
                        concrete_line_id, record.offset
                    ),
                    context: None,
                });
            }
            let pack_end = u64::from(record.offset) + u64::from(record.byte_length);
            if pack_end > bin_raw.len() as u64 {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::IndexPackCross,
                    check: check.clone(),
                    reason: "OUT_OF_BOUNDS".to_owned(),
                    message: format!(
                        ".idx record concreteLineId={}: offset+byteLength={} exceeds .bin file size {}",
                        concrete_line_id,
                        pack_end,
                        bin_raw.len()
                    ),
                    context: None,
                });
                continue;
            }
            if let Some(action_count) = action_counts.get(&record.action_schema_id) {
                let expected_pack_len =
                    u32::from(record.hand_count) * (5 + action_count.saturating_mul(8));
                if record.byte_length != expected_pack_len {
                    failures.push(VerifyFailure {
                        layer: VerifyLayer::IndexPackCross,
                        check: check.clone(),
                        reason: "PACK_SIZE_MISMATCH".to_owned(),
                        message: format!(
                            ".idx record concreteLineId={}: byteLength={} != handCount*(5+{}*8)={expected_pack_len}",
                            concrete_line_id, record.byte_length, action_count
                        ),
                        context: None,
                    });
                }
            }

            let pack_start = record.offset as usize;
            let pack_end = pack_end as usize;
            let pack = &bin_raw[pack_start..pack_end];
            if options.verify_checksums {
                let actual_crc = crc32c(pack);
                if actual_crc != record.checksum {
                    failures.push(VerifyFailure {
                        layer: VerifyLayer::IndexPackCross,
                        check: check.clone(),
                        reason: "CHECKSUM_MISMATCH".to_owned(),
                        message: format!(
                            ".idx record concreteLineId={}: stored CRC {} != computed {actual_crc}",
                            concrete_line_id, record.checksum
                        ),
                        context: None,
                    });
                }
            }
            validate_pack_hand_ids(&check, concrete_line_id, record.hand_count, pack, failures);
        }
    }
}

fn validate_pack_hand_ids(
    check: &str,
    concrete_line_id: u32,
    hand_count: u16,
    pack: &[u8],
    failures: &mut Vec<VerifyFailure>,
) {
    if pack.len() < hand_count as usize {
        return;
    }
    let mut previous = None;
    for (index, hand_id) in pack[..hand_count as usize].iter().copied().enumerate() {
        if hand_id > 168 {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexPackCross,
                check: check.to_owned(),
                reason: "INVALID_HAND_ID".to_owned(),
                message: format!(
                    ".idx record concreteLineId={concrete_line_id}: handId={hand_id} at index {index} is out of range [0, 168]"
                ),
                context: None,
            });
        }
        if previous.is_some_and(|value| hand_id <= value) {
            failures.push(VerifyFailure {
                layer: VerifyLayer::IndexPackCross,
                check: check.to_owned(),
                reason: "HAND_ID_NOT_SORTED".to_owned(),
                message: format!(
                    ".idx record concreteLineId={concrete_line_id}: handIds not strictly increasing at index {index}"
                ),
                context: None,
            });
            break;
        }
        previous = Some(hand_id);
    }
}

fn build_dimension_details(
    dir: &std::path::Path,
    manifest: &BuildManifest,
    record_counts: &HashMap<String, u32>,
    failures: &[VerifyFailure],
) -> Vec<DimensionVerifyDetail> {
    manifest
        .dimensions
        .iter()
        .map(|dimension| {
            let key = dimension_key(dimension);
            let check = format!("dimension:{key}");
            let header_failures = failures
                .iter()
                .filter(|failure| {
                    failure.check == check
                        && matches!(
                            failure.layer,
                            VerifyLayer::IndexHeader
                                | VerifyLayer::PackHeader
                                | VerifyLayer::FileExistence
                        )
                })
                .count();
            let index_pack_cross_failures = failures
                .iter()
                .filter(|failure| {
                    failure.check == check && failure.layer == VerifyLayer::IndexPackCross
                })
                .count();
            DimensionVerifyDetail {
                strategy: dimension.strategy.clone(),
                player_count: dimension.player_count,
                depth_bb: dimension.depth_bb,
                checked: dimension.status.as_deref() != Some("failed"),
                index_records: record_counts.get(&key).copied().unwrap_or_default(),
                bin_file_size_bytes: dimension
                    .bin_file
                    .as_ref()
                    .and_then(|file| fs::metadata(dir.join(file)).ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or_default(),
                idx_file_size_bytes: dimension
                    .idx_file
                    .as_ref()
                    .and_then(|file| fs::metadata(dir.join(file)).ok())
                    .map(|metadata| metadata.len())
                    .unwrap_or_default(),
                header_failures,
                index_pack_cross_failures,
                source_cross_failures: None,
                source_cross_records: None,
            }
        })
        .collect()
}

fn validate_bin_header(header: &[u8]) -> Option<String> {
    if header[0..4] != PFSP_MAGIC[..] {
        return Some("Invalid ranges.bin magic, expected PFSP".to_owned());
    }
    let version = u16_from_le(&header[4..6]);
    if version != 1 {
        return Some(format!("Unsupported PFSP version: {version}"));
    }
    if header[6] != 1 {
        return Some("Unsupported endian, expected little-endian".to_owned());
    }
    if header[7] != 1 {
        return Some("Unsupported float type, expected float32".to_owned());
    }
    if header[8] != 1 {
        return Some("Unsupported layout, expected sparse hand-major v1".to_owned());
    }
    if header[9] != 0 {
        return Some("Unsupported compression, expected none".to_owned());
    }
    let header_size = u16_from_le(&header[10..12]);
    if header_size as usize != PFSP_HEADER_SIZE {
        return Some(format!("Unsupported header size: {header_size}"));
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct RawIdxRecord {
    action_schema_id: u32,
    hand_count: u16,
    offset: u32,
    byte_length: u32,
    checksum: u32,
}

fn decode_idx_record(bytes: &[u8]) -> RawIdxRecord {
    RawIdxRecord {
        action_schema_id: u32_from_le(&bytes[0..4]),
        hand_count: u16_from_le(&bytes[4..6]),
        offset: u32_from_le(&bytes[6..10]),
        byte_length: u32_from_le(&bytes[10..14]),
        checksum: u32_from_le(&bytes[14..18]),
    }
}

fn successful_dimensions(manifest: &BuildManifest) -> impl Iterator<Item = &ManifestDimension> {
    manifest
        .dimensions
        .iter()
        .filter(|dimension| dimension.status.as_deref() != Some("failed"))
}

fn dimension_key(dimension: &ManifestDimension) -> String {
    format!(
        "{}:{}max:{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

fn u32_from_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn u16_from_le(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn write_reports(
    report: &RangeStrataVerifyReport,
    options: &StandaloneVerifyOptions,
) -> Result<(), ToolError> {
    if let Some(path) = &options.out_path {
        write_json_report(report, path)?;
    }
    if let Some(path) = &options.md_path {
        write_markdown_report(report, path)?;
    }
    Ok(())
}
