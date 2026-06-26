use std::collections::{HashMap, HashSet};
use std::path::Path;

use range_store_core::crc32c::crc32c;

use crate::manifest::BuildManifest;
use crate::naming::{get_concrete_lines_table_name, get_drill_scenario_table_name};
use crate::sqlite::Connection;
use crate::verifier::report::{VerifyFailure, VerifyLayer};

#[derive(Debug, Clone, Default)]
pub struct CatalogInfo {
    pub action_counts: HashMap<u32, u32>,
    pub valid_action_schema_ids: HashSet<u32>,
}

pub fn check_catalog(dir: &Path, manifest: &BuildManifest) -> (CatalogInfo, Vec<VerifyFailure>) {
    let mut failures = Vec::new();
    let mut info = CatalogInfo::default();
    let meta_path = dir.join("meta.db");
    let connection = match Connection::open(&meta_path, true) {
        Ok(connection) => connection,
        Err(error) => {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "open".to_owned(),
                reason: "IO_ERROR".to_owned(),
                message: format!("Cannot open meta.db at {}: {error}", meta_path.display()),
                context: None,
            });
            return (info, failures);
        }
    };

    let table_names = match load_table_names(&connection) {
        Ok(table_names) => table_names,
        Err(error) => {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "sqlite_master".to_owned(),
                reason: "IO_ERROR".to_owned(),
                message: error,
                context: None,
            });
            return (info, failures);
        }
    };

    check_build_info(&connection, &table_names, &mut failures);
    check_action_schemas(&connection, &table_names, &mut info, &mut failures);
    check_metadata_tables(manifest, &table_names, &mut failures);

    (info, failures)
}

fn load_table_names(connection: &Connection) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|error| error.to_string())?;
    statement.start(&[]).map_err(|error| error.to_string())?;
    let mut table_names = HashSet::new();
    while statement.step_row().map_err(|error| error.to_string())? {
        table_names.insert(
            statement
                .column_text(0)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(table_names)
}

fn check_build_info(
    connection: &Connection,
    table_names: &HashSet<String>,
    failures: &mut Vec<VerifyFailure>,
) {
    if !table_names.contains("build_info") {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "build_info".to_owned(),
            reason: "MISSING_TABLE".to_owned(),
            message: "Required table 'build_info' not found in meta.db".to_owned(),
            context: None,
        });
        return;
    }

    let mut statement = match connection
        .prepare("SELECT key, value FROM build_info WHERE key IN ('built_at', 'source_checksum')")
    {
        Ok(statement) => statement,
        Err(error) => {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "build_info".to_owned(),
                reason: "IO_ERROR".to_owned(),
                message: error.to_string(),
                context: None,
            });
            return;
        }
    };
    if let Err(error) = statement.start(&[]) {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "build_info".to_owned(),
            reason: "IO_ERROR".to_owned(),
            message: error.to_string(),
            context: None,
        });
        return;
    }

    let mut have_built_at = false;
    let mut have_source_checksum = false;
    while matches!(statement.step_row(), Ok(true)) {
        let key = statement.column_text(0).unwrap_or_default();
        let value = statement.column_text(1).unwrap_or_default();
        if key == "built_at" && !value.is_empty() {
            have_built_at = true;
        }
        if key == "source_checksum" && !value.is_empty() {
            have_source_checksum = true;
        }
    }
    if !have_built_at {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "build_info.built_at".to_owned(),
            reason: "MISSING_ROW".to_owned(),
            message: "build_info missing 'built_at' entry".to_owned(),
            context: None,
        });
    }
    if !have_source_checksum {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "build_info.source_checksum".to_owned(),
            reason: "MISSING_ROW".to_owned(),
            message: "build_info missing 'source_checksum' entry".to_owned(),
            context: None,
        });
    }
}

fn check_action_schemas(
    connection: &Connection,
    table_names: &HashSet<String>,
    info: &mut CatalogInfo,
    failures: &mut Vec<VerifyFailure>,
) {
    if !table_names.contains("action_schemas") {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "action_schemas".to_owned(),
            reason: "MISSING_TABLE".to_owned(),
            message: "Required table 'action_schemas' not found in meta.db".to_owned(),
            context: None,
        });
        return;
    }

    let mut statement = match connection.prepare(
        "SELECT id, action_count, action_blob, checksum, schema_key
         FROM action_schemas
         ORDER BY id",
    ) {
        Ok(statement) => statement,
        Err(error) => {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "action_schemas".to_owned(),
                reason: "IO_ERROR".to_owned(),
                message: error.to_string(),
                context: None,
            });
            return;
        }
    };
    if let Err(error) = statement.start(&[]) {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "action_schemas".to_owned(),
            reason: "IO_ERROR".to_owned(),
            message: error.to_string(),
            context: None,
        });
        return;
    }

    let mut row_count = 0usize;
    while matches!(statement.step_row(), Ok(true)) {
        row_count += 1;
        let id = match statement.column_u32(0) {
            Ok(id) => id,
            Err(error) => {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::Catalog,
                    check: "action_schemas".to_owned(),
                    reason: "INVALID_FORMAT".to_owned(),
                    message: error.to_string(),
                    context: None,
                });
                continue;
            }
        };
        let action_count = statement.column_u32(1).unwrap_or_default();
        let action_blob = statement.column_blob(2);
        let checksum = statement.column_i64(3) as u32;
        let schema_key = statement.column_text(4).unwrap_or_default();
        info.valid_action_schema_ids.insert(id);
        info.action_counts.insert(id, action_count);

        let expected_blob_len = action_count as usize * 9;
        if action_blob.len() != expected_blob_len {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "action_schemas".to_owned(),
                reason: "INVALID_FORMAT".to_owned(),
                message: format!(
                    "action_schema id={id}: blob length {} != action_count * 9 ({expected_blob_len})",
                    action_blob.len()
                ),
                context: None,
            });
        }
        if !(1..=32).contains(&action_count) {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "action_schemas".to_owned(),
                reason: "INVALID_ARGUMENT".to_owned(),
                message: format!(
                    "action_schema id={id}: action_count={action_count} out of range [1, 32]"
                ),
                context: None,
            });
        }
        let actual_checksum = crc32c(&action_blob);
        if actual_checksum != checksum {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "action_schemas".to_owned(),
                reason: "CHECKSUM_MISMATCH".to_owned(),
                message: format!(
                    "action_schema id={id}: stored checksum {checksum} != computed {actual_checksum}"
                ),
                context: None,
            });
        }
        let hex = to_hex(&action_blob);
        if hex != schema_key {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: "action_schemas".to_owned(),
                reason: "SCHEMA_KEY_MISMATCH".to_owned(),
                message: format!(
                    "action_schema id={id}: stored schema_key does not match hex(blob)"
                ),
                context: None,
            });
        }
    }

    if row_count == 0 {
        failures.push(VerifyFailure {
            layer: VerifyLayer::Catalog,
            check: "action_schemas".to_owned(),
            reason: "EMPTY".to_owned(),
            message: "action_schemas table is empty".to_owned(),
            context: None,
        });
    }
}

fn check_metadata_tables(
    manifest: &BuildManifest,
    table_names: &HashSet<String>,
    failures: &mut Vec<VerifyFailure>,
) {
    let mut seen_strategies = HashSet::new();
    for dimension in &manifest.dimensions {
        if dimension.status.as_deref() == Some("failed") {
            continue;
        }
        if seen_strategies.insert(dimension.strategy.clone()) {
            let drill_table = get_drill_scenario_table_name(&dimension.strategy);
            if !table_names.contains(&drill_table) {
                failures.push(VerifyFailure {
                    layer: VerifyLayer::Catalog,
                    check: format!("drill:{}", dimension.strategy),
                    reason: "MISSING_TABLE".to_owned(),
                    message: format!("Expected drill table \"{drill_table}\" not found"),
                    context: None,
                });
            }
        }

        let concrete_table = get_concrete_lines_table_name(
            &dimension.strategy,
            dimension.player_count,
            dimension.depth_bb,
        );
        if !table_names.contains(&concrete_table) {
            failures.push(VerifyFailure {
                layer: VerifyLayer::Catalog,
                check: format!(
                    "concrete_lines:{}:{}max:{}BB",
                    dimension.strategy, dimension.player_count, dimension.depth_bb
                ),
                reason: "MISSING_TABLE".to_owned(),
                message: format!("Expected concrete_lines table \"{concrete_table}\" not found"),
                context: None,
            });
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
