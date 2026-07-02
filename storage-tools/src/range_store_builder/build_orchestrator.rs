use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use range_store_core::crc32c::crc32c;
use range_store_core::dimension::{
    discover_dimensions, get_bin_file_name, get_drill_scenario_table_name, get_idx_file_name,
    quote_identifier, DimensionSpec,
};
use range_store_core::hole_cards::get_hand_id;
use range_store_core::manifest::{BuildManifest, ManifestDimension};
use range_store_core::sqlite::{Connection, Value};
use range_store_core::types::{IDX_HEADER_SIZE, IDX_RECORD_SIZE, PFSP_HEADER_SIZE};
use serde::{Deserialize, Serialize};

use crate::errors::ToolError;

const BUILD_STATE_FILE: &str = "build-state.json";
const BUILD_STATE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimensions: Vec<DimensionSpec>,
    pub max_concrete_lines_per_dimension: Option<usize>,
    pub overwrite: bool,
    pub resume: bool,
}

#[derive(Debug, Clone)]
pub struct BuildSummary {
    pub manifest_path: PathBuf,
    pub dimensions: Vec<ManifestDimension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildState {
    version: u32,
    source_db: String,
    source_db_checksum: String,
    output_dir: String,
    built_at: String,
    updated_at: String,
    max_concrete_lines_per_dimension: Option<usize>,
    dimensions: Vec<BuildStateDimension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildStateDimension {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    status: String,
    error: Option<String>,
    concrete_line_count: Option<u32>,
    pack_count: Option<u32>,
    bin_file: Option<String>,
    idx_file: Option<String>,
    bin_file_size_bytes: Option<u64>,
    idx_file_size_bytes: Option<u64>,
    bin_file_checksum: Option<String>,
    idx_file_checksum: Option<String>,
    completed_at: Option<String>,
}

#[derive(Debug, Clone)]
struct RangeRow {
    concrete_line_id: u32,
    hole_cards: String,
    action_name: String,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ActionKey {
    action_type: u8,
    action_size_bits: u64,
    amount_bb_bits: u64,
}

impl ActionKey {
    fn new(action_type: u8, action_size: f64, amount_bb: f64) -> Self {
        Self {
            action_type,
            action_size_bits: action_size.to_bits(),
            amount_bb_bits: amount_bb.to_bits(),
        }
    }

    fn action_size(self) -> f64 {
        f64::from_bits(self.action_size_bits)
    }

    fn amount_bb(self) -> f64 {
        f64::from_bits(self.amount_bb_bits)
    }
}

pub fn build_store(options: &BuildOptions) -> Result<BuildSummary, ToolError> {
    if options.resume && options.overwrite {
        return Err(ToolError::invalid_argument(
            "--resume and --overwrite cannot be used together",
        ));
    }
    if !options.source_db.is_file() {
        return Err(ToolError::build(format!(
            "Source DB not found: {}",
            options.source_db.display()
        )));
    }

    let source = Connection::open(&options.source_db, true)?;
    let discovered = discover_dimensions(&source)?;
    let dimensions = select_dimensions(discovered, &options.dimensions)?;
    if dimensions.is_empty() {
        return Err(ToolError::build("No range dimensions selected"));
    }

    let source_db_checksum = sha256_file(&options.source_db)?;
    let meta_path = options.out_dir.join("meta.db");
    let state_path = options.out_dir.join(BUILD_STATE_FILE);
    let mut state = prepare_build_state(
        options,
        &dimensions,
        &source_db_checksum,
        &state_path,
        &meta_path,
    )?;
    let meta = Connection::open(&meta_path, false)?;

    let mut schema_ids_by_key = HashMap::new();
    let mut manifest_dimensions = Vec::with_capacity(dimensions.len());
    for dimension in &dimensions {
        if let Some(completed) = completed_state_dimension(&state, dimension)? {
            manifest_dimensions.push(completed);
            continue;
        }
        let manifest_dimension = build_dimension(
            &source,
            &meta,
            &options.out_dir,
            dimension,
            options.max_concrete_lines_per_dimension,
            &mut schema_ids_by_key,
        )?;
        mark_state_dimension_completed(&mut state, dimension, &manifest_dimension)?;
        state.updated_at = utc_now_iso8601()?;
        write_build_state(&state_path, &state)?;
        manifest_dimensions.push(manifest_dimension);
    }
    drop(meta);

    let mut files = vec!["meta.db".to_owned()];
    for dimension in &manifest_dimensions {
        if let Some(bin_file) = &dimension.bin_file {
            files.push(bin_file.clone());
        }
        if let Some(idx_file) = &dimension.idx_file {
            files.push(idx_file.clone());
        }
    }
    let manifest = BuildManifest {
        format: "PFSP".to_owned(),
        version: 1,
        source_db_checksum,
        built_at: state.built_at.clone(),
        dimensions: manifest_dimensions.clone(),
        files,
    };
    let manifest_path = options.out_dir.join("manifest.json");
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| ToolError::build(error.to_string()))?;
    fs::write(&manifest_path, format!("{manifest_json}\n"))?;

    Ok(BuildSummary {
        manifest_path,
        dimensions: manifest_dimensions,
    })
}

fn prepare_build_state(
    options: &BuildOptions,
    dimensions: &[DimensionSpec],
    source_db_checksum: &str,
    state_path: &Path,
    meta_path: &Path,
) -> Result<BuildState, ToolError> {
    if options.resume && state_path.is_file() {
        let state = load_build_state(state_path)?;
        validate_build_state(&state, options, dimensions, source_db_checksum)?;
        if !meta_path.is_file() {
            return Err(ToolError::build(format!(
                "Cannot resume: missing meta.db at {}",
                meta_path.display()
            )));
        }
        return Ok(state);
    }

    if options.resume {
        prepare_resumable_output_dir(&options.out_dir)?;
    } else {
        prepare_output_dir(&options.out_dir, options.overwrite)?;
    }

    let built_at = utc_now_iso8601()?;
    let meta = Connection::open(meta_path, false)?;
    init_meta_db(&meta, dimensions)?;
    let source = Connection::open(&options.source_db, true)?;
    copy_metadata(&source, &meta, dimensions)?;
    meta.execute(
        "INSERT OR REPLACE INTO build_info(key, value) VALUES ('source_checksum', ?1)",
        &[Value::from(source_db_checksum)],
    )?;
    meta.execute(
        "INSERT OR REPLACE INTO build_info(key, value) VALUES ('built_at', ?1)",
        &[Value::from(built_at.as_str())],
    )?;
    drop(meta);

    let state = BuildState {
        version: BUILD_STATE_VERSION,
        source_db: options.source_db.display().to_string(),
        source_db_checksum: source_db_checksum.to_owned(),
        output_dir: options.out_dir.display().to_string(),
        built_at: built_at.clone(),
        updated_at: built_at,
        max_concrete_lines_per_dimension: options.max_concrete_lines_per_dimension,
        dimensions: dimensions
            .iter()
            .map(|dimension| BuildStateDimension {
                strategy: dimension.strategy.clone(),
                player_count: dimension.player_count,
                depth_bb: dimension.depth_bb,
                status: "pending".to_owned(),
                error: None,
                concrete_line_count: None,
                pack_count: None,
                bin_file: None,
                idx_file: None,
                bin_file_size_bytes: None,
                idx_file_size_bytes: None,
                bin_file_checksum: None,
                idx_file_checksum: None,
                completed_at: None,
            })
            .collect(),
    };
    write_build_state(state_path, &state)?;
    Ok(state)
}

fn prepare_resumable_output_dir(out_dir: &Path) -> Result<(), ToolError> {
    if out_dir.exists() {
        let has_entries = fs::read_dir(out_dir)?.next().transpose()?.is_some();
        if has_entries {
            return Err(ToolError::build(format!(
                "Cannot resume: output directory is not empty and {} is missing. Pass --overwrite to rebuild from scratch.",
                BUILD_STATE_FILE
            )));
        }
    }
    fs::create_dir_all(out_dir)?;
    Ok(())
}

fn load_build_state(path: &Path) -> Result<BuildState, ToolError> {
    let raw = fs::read_to_string(path)?;
    serde_json::from_str(&raw).map_err(|error| {
        ToolError::invalid_format(format!("Failed to parse {}: {error}", path.display()))
    })
}

fn write_build_state(path: &Path, state: &BuildState) -> Result<(), ToolError> {
    let raw = serde_json::to_string_pretty(state)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, format!("{raw}\n"))?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(tmp_path, path)?;
    Ok(())
}

fn validate_build_state(
    state: &BuildState,
    options: &BuildOptions,
    dimensions: &[DimensionSpec],
    source_db_checksum: &str,
) -> Result<(), ToolError> {
    if state.version != BUILD_STATE_VERSION {
        return Err(ToolError::build(format!(
            "Cannot resume: unsupported build-state version {}",
            state.version
        )));
    }
    if state.source_db_checksum != source_db_checksum {
        return Err(ToolError::build(
            "Cannot resume: source database checksum does not match build-state.json",
        ));
    }
    if state.max_concrete_lines_per_dimension != options.max_concrete_lines_per_dimension {
        return Err(ToolError::build(
            "Cannot resume: --max-concrete-lines differs from build-state.json",
        ));
    }
    let state_dimensions: Vec<_> = state
        .dimensions
        .iter()
        .map(|dimension| {
            (
                dimension.strategy.as_str(),
                dimension.player_count,
                dimension.depth_bb,
            )
        })
        .collect();
    let expected_dimensions: Vec<_> = dimensions
        .iter()
        .map(|dimension| {
            (
                dimension.strategy.as_str(),
                dimension.player_count,
                dimension.depth_bb,
            )
        })
        .collect();
    if state_dimensions != expected_dimensions {
        return Err(ToolError::build(
            "Cannot resume: selected dimensions differ from build-state.json",
        ));
    }
    for dimension in &state.dimensions {
        if dimension.status == "completed" {
            validate_completed_state_dimension(&options.out_dir, dimension)?;
        }
    }
    Ok(())
}

fn validate_completed_state_dimension(
    out_dir: &Path,
    dimension: &BuildStateDimension,
) -> Result<(), ToolError> {
    let bin_file = dimension.bin_file.as_ref().ok_or_else(|| {
        ToolError::build("Cannot resume: completed dimension is missing bin_file")
    })?;
    let idx_file = dimension.idx_file.as_ref().ok_or_else(|| {
        ToolError::build("Cannot resume: completed dimension is missing idx_file")
    })?;
    let bin_path = out_dir.join(bin_file);
    let idx_path = out_dir.join(idx_file);
    validate_state_file(
        &bin_path,
        dimension.bin_file_size_bytes,
        dimension.bin_file_checksum.as_deref(),
    )?;
    validate_state_file(
        &idx_path,
        dimension.idx_file_size_bytes,
        dimension.idx_file_checksum.as_deref(),
    )?;
    Ok(())
}

fn validate_state_file(
    path: &Path,
    expected_size: Option<u64>,
    expected_checksum: Option<&str>,
) -> Result<(), ToolError> {
    let metadata = fs::metadata(path).map_err(|error| {
        ToolError::build(format!(
            "Cannot resume: missing or unreadable file {}: {error}",
            path.display()
        ))
    })?;
    if Some(metadata.len()) != expected_size {
        return Err(ToolError::build(format!(
            "Cannot resume: file size mismatch for {}",
            path.display()
        )));
    }
    if Some(sha256_file(path)?.as_str()) != expected_checksum {
        return Err(ToolError::build(format!(
            "Cannot resume: checksum mismatch for {}",
            path.display()
        )));
    }
    Ok(())
}

fn completed_state_dimension(
    state: &BuildState,
    dimension: &DimensionSpec,
) -> Result<Option<ManifestDimension>, ToolError> {
    let Some(entry) = state.dimensions.iter().find(|entry| {
        entry.strategy == dimension.strategy
            && entry.player_count == dimension.player_count
            && entry.depth_bb == dimension.depth_bb
    }) else {
        return Err(ToolError::build(format!(
            "Cannot resume: dimension missing from build-state.json: {}:{}:{}",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        )));
    };
    if entry.status != "completed" {
        return Ok(None);
    }
    Ok(Some(ManifestDimension {
        strategy: entry.strategy.clone(),
        player_count: entry.player_count,
        depth_bb: entry.depth_bb,
        concrete_line_count: entry
            .concrete_line_count
            .ok_or_else(|| ToolError::build("Cannot resume: missing concrete_line_count"))?,
        pack_count: entry
            .pack_count
            .ok_or_else(|| ToolError::build("Cannot resume: missing pack_count"))?,
        status: Some("success".to_owned()),
        error: None,
        bin_file: entry.bin_file.clone(),
        idx_file: entry.idx_file.clone(),
        bin_file_size_bytes: entry.bin_file_size_bytes,
        idx_file_size_bytes: entry.idx_file_size_bytes,
    }))
}

fn mark_state_dimension_completed(
    state: &mut BuildState,
    dimension: &DimensionSpec,
    manifest: &ManifestDimension,
) -> Result<(), ToolError> {
    let entry = state
        .dimensions
        .iter_mut()
        .find(|entry| {
            entry.strategy == dimension.strategy
                && entry.player_count == dimension.player_count
                && entry.depth_bb == dimension.depth_bb
        })
        .ok_or_else(|| {
            ToolError::build(format!(
                "Cannot update build-state: dimension missing: {}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ))
        })?;
    let bin_file = manifest
        .bin_file
        .as_ref()
        .ok_or_else(|| ToolError::build("Completed dimension is missing bin_file"))?;
    let idx_file = manifest
        .idx_file
        .as_ref()
        .ok_or_else(|| ToolError::build("Completed dimension is missing idx_file"))?;
    let bin_path = Path::new(&state.output_dir).join(bin_file);
    let idx_path = Path::new(&state.output_dir).join(idx_file);
    entry.status = "completed".to_owned();
    entry.error = None;
    entry.concrete_line_count = Some(manifest.concrete_line_count);
    entry.pack_count = Some(manifest.pack_count);
    entry.bin_file = Some(bin_file.clone());
    entry.idx_file = Some(idx_file.clone());
    entry.bin_file_size_bytes = manifest.bin_file_size_bytes;
    entry.idx_file_size_bytes = manifest.idx_file_size_bytes;
    entry.bin_file_checksum = Some(sha256_file(&bin_path)?);
    entry.idx_file_checksum = Some(sha256_file(&idx_path)?);
    entry.completed_at = Some(utc_now_iso8601()?);
    Ok(())
}

fn select_dimensions(
    discovered: Vec<DimensionSpec>,
    requested: &[DimensionSpec],
) -> Result<Vec<DimensionSpec>, ToolError> {
    if requested.is_empty() {
        return Ok(discovered);
    }
    let available: HashSet<_> = discovered.iter().cloned().collect();
    for dimension in requested {
        if !available.contains(dimension) {
            return Err(ToolError::build(format!(
                "Requested dimension not found: {}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            )));
        }
    }
    Ok(discovered
        .into_iter()
        .filter(|dimension| requested.contains(dimension))
        .collect())
}

fn prepare_output_dir(out_dir: &Path, overwrite: bool) -> Result<(), ToolError> {
    if out_dir.exists() {
        let has_entries = fs::read_dir(out_dir)?.next().transpose()?.is_some();
        if has_entries && !overwrite {
            return Err(ToolError::build(format!(
                "Output directory is not empty: {}. Pass --overwrite to replace it.",
                out_dir.display()
            )));
        }
        if has_entries {
            fs::remove_dir_all(out_dir)?;
        }
    }
    fs::create_dir_all(out_dir)?;
    Ok(())
}

fn init_meta_db(connection: &Connection, dimensions: &[DimensionSpec]) -> Result<(), ToolError> {
    connection.exec(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE build_info (
           key TEXT PRIMARY KEY,
           value TEXT NOT NULL
         );
         CREATE TABLE action_schemas (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           action_count INTEGER NOT NULL,
           action_blob BLOB NOT NULL,
           checksum INTEGER NOT NULL,
           schema_key TEXT NOT NULL UNIQUE
         );
         CREATE TABLE dimension_action_schemas (
           strategy TEXT NOT NULL,
           player_count INTEGER NOT NULL,
           depth_bb INTEGER NOT NULL,
           action_schema_id INTEGER NOT NULL,
           PRIMARY KEY (strategy, player_count, depth_bb, action_schema_id)
         );",
    )?;

    let strategies: BTreeSet<_> = dimensions
        .iter()
        .map(|dimension| dimension.strategy.as_str())
        .collect();
    for strategy in strategies {
        let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
        connection.exec(&format!(
            "CREATE TABLE {table} (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               drill_name TEXT NOT NULL,
               abstract_line TEXT NOT NULL,
               player_count INTEGER NOT NULL,
               drill_depth INTEGER NOT NULL DEFAULT 100,
               UNIQUE(drill_name, player_count, drill_depth, abstract_line)
             );"
        ))?;
    }
    for dimension in dimensions {
        let raw_table = dimension.concrete_table();
        let table = quote_identifier(&raw_table)?;
        let concrete_line_index = quote_identifier(&format!("idx_{raw_table}_concrete_line"))?;
        connection.exec(&format!(
            "CREATE TABLE {table} (
               concrete_line_id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL,
               UNIQUE(abstract_line, concrete_line)
             );
             CREATE INDEX {concrete_line_index}
               ON {table}(concrete_line);"
        ))?;
    }
    Ok(())
}

fn copy_metadata(
    source: &Connection,
    target: &Connection,
    dimensions: &[DimensionSpec],
) -> Result<(), ToolError> {
    target.exec("BEGIN")?;
    let result = (|| {
        let strategies: BTreeSet<_> = dimensions
            .iter()
            .map(|dimension| dimension.strategy.as_str())
            .collect();
        for strategy in strategies {
            copy_drill_lines(source, target, strategy)?;
        }
        for dimension in dimensions {
            copy_concrete_lines(source, target, dimension)?;
        }
        Ok::<(), ToolError>(())
    })();
    finish_transaction(target, result)
}

fn finish_transaction(
    connection: &Connection,
    result: Result<(), ToolError>,
) -> Result<(), ToolError> {
    match result {
        Ok(()) => connection.exec("COMMIT").map_err(ToolError::from),
        Err(error) => {
            let _ = connection.exec("ROLLBACK");
            Err(error)
        }
    }
}

fn copy_drill_lines(
    source: &Connection,
    target: &Connection,
    strategy: &str,
) -> Result<(), ToolError> {
    let raw_table = get_drill_scenario_table_name(strategy);
    let mut exists_statement = source.prepare(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
         )",
    )?;
    exists_statement.start(&[Value::from(raw_table.as_str())])?;
    let exists = exists_statement.step_row()? && exists_statement.column_i64(0) != 0;
    if !exists {
        return Ok(());
    }
    let table = quote_identifier(&raw_table)?;
    let mut select = source.prepare(&format!(
        "SELECT drill_name, abstract_line, player_count, depth
         FROM {table}
         ORDER BY id"
    ))?;
    select.start(&[])?;
    let mut insert = target.prepare(&format!(
        "INSERT OR IGNORE INTO {table}(
           drill_name, abstract_line, player_count, drill_depth
         ) VALUES (?1, ?2, ?3, ?4)"
    ))?;
    while select.step_row()? {
        insert.execute(&[
            Value::from(select.column_text(0)?),
            Value::from(select.column_text(1)?),
            Value::from(select.column_u32(2)?),
            Value::from(select.column_u32(3)?),
        ])?;
    }
    Ok(())
}

fn copy_concrete_lines(
    source: &Connection,
    target: &Connection,
    dimension: &DimensionSpec,
) -> Result<(), ToolError> {
    let table = quote_identifier(&dimension.concrete_table())?;
    let mut select = source.prepare(&format!(
        "SELECT id, abstract_line, concrete_line FROM {table} ORDER BY id"
    ))?;
    select.start(&[])?;
    let mut insert = target.prepare(&format!(
        "INSERT OR IGNORE INTO {table}(
           concrete_line_id, abstract_line, concrete_line
         ) VALUES (?1, ?2, ?3)"
    ))?;
    while select.step_row()? {
        insert.execute(&[
            Value::from(select.column_u32(0)?),
            Value::from(select.column_text(1)?),
            Value::from(select.column_text(2)?),
        ])?;
    }
    Ok(())
}

fn build_dimension(
    source: &Connection,
    meta: &Connection,
    out_dir: &Path,
    dimension: &DimensionSpec,
    max_concrete_lines: Option<usize>,
    schema_ids_by_key: &mut HashMap<String, u32>,
) -> Result<ManifestDimension, ToolError> {
    let bin_name = get_bin_file_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    );
    let idx_name = get_idx_file_name(
        &dimension.strategy,
        dimension.player_count,
        dimension.depth_bb,
    );
    let bin_path = out_dir.join(&bin_name);
    let idx_path = out_dir.join(&idx_name);
    let bin_tmp = out_dir.join(format!("{bin_name}.tmp"));
    let idx_tmp = out_dir.join(format!("{idx_name}.tmp"));

    remove_if_exists(&bin_tmp)?;
    remove_if_exists(&idx_tmp)?;
    remove_if_exists(&bin_path)?;
    remove_if_exists(&idx_path)?;

    let result = build_dimension_files(
        source,
        meta,
        dimension,
        max_concrete_lines,
        schema_ids_by_key,
        &bin_tmp,
        &idx_tmp,
    );
    if let Err(error) = result {
        let _ = fs::remove_file(&bin_tmp);
        let _ = fs::remove_file(&idx_tmp);
        return Err(error);
    }
    let (pack_count, concrete_line_count) = result?;
    fs::rename(&bin_tmp, &bin_path)?;
    fs::rename(&idx_tmp, &idx_path)?;

    Ok(ManifestDimension {
        strategy: dimension.strategy.clone(),
        player_count: dimension.player_count,
        depth_bb: dimension.depth_bb,
        concrete_line_count,
        pack_count,
        status: Some("success".to_owned()),
        error: None,
        bin_file: Some(bin_name),
        idx_file: Some(idx_name),
        bin_file_size_bytes: Some(fs::metadata(bin_path)?.len()),
        idx_file_size_bytes: Some(fs::metadata(idx_path)?.len()),
    })
}

#[allow(clippy::too_many_arguments)]
fn build_dimension_files(
    source: &Connection,
    meta: &Connection,
    dimension: &DimensionSpec,
    max_concrete_lines: Option<usize>,
    schema_ids_by_key: &mut HashMap<String, u32>,
    bin_tmp: &Path,
    idx_tmp: &Path,
) -> Result<(u32, u32), ToolError> {
    let mut bin = create_new_file(bin_tmp)?;
    let mut idx = create_new_file(idx_tmp)?;
    write_bin_header(&mut bin)?;
    write_idx_header(&mut idx, 0)?;
    let mut bin_offset = PFSP_HEADER_SIZE as u32;
    let mut pack_count = 0u32;
    let mut current_line_id = None;
    let mut current_rows = Vec::new();
    let mut dimension_schema_ids = HashSet::new();

    let range_table = quote_identifier(&dimension.range_table())?;
    let mut statement = source.prepare(&format!(
        "SELECT concrete_line_id, hole_cards, action_name, action_size,
                amount_bb, frequency, hand_ev
         FROM {range_table}
         ORDER BY concrete_line_id, hole_cards, action_name"
    ))?;
    statement.start(&[])?;
    meta.exec("BEGIN")?;

    let build_result = (|| {
        while statement.step_row()? {
            let range_row = RangeRow {
                concrete_line_id: statement.column_u32(0)?,
                hole_cards: statement.column_text(1)?,
                action_name: statement.column_text(2)?,
                action_size: statement.column_f64(3),
                amount_bb: statement.column_f64(4),
                frequency: statement.column_f64(5),
                hand_ev: statement.column_optional_f64(6),
            };
            if current_line_id.is_none() {
                current_line_id = Some(range_row.concrete_line_id);
            }
            if current_line_id != Some(range_row.concrete_line_id) {
                flush_pack(
                    meta,
                    schema_ids_by_key,
                    &mut dimension_schema_ids,
                    current_line_id.expect("current line is set"),
                    &current_rows,
                    &mut bin,
                    &mut idx,
                    &mut bin_offset,
                )?;
                pack_count += 1;
                if max_concrete_lines.is_some_and(|limit| pack_count as usize >= limit) {
                    current_line_id = None;
                    current_rows.clear();
                    break;
                }
                current_line_id = Some(range_row.concrete_line_id);
                current_rows.clear();
            }
            current_rows.push(range_row);
        }
        if let Some(concrete_line_id) = current_line_id.filter(|_| !current_rows.is_empty()) {
            flush_pack(
                meta,
                schema_ids_by_key,
                &mut dimension_schema_ids,
                concrete_line_id,
                &current_rows,
                &mut bin,
                &mut idx,
                &mut bin_offset,
            )?;
            pack_count += 1;
        }

        for action_schema_id in dimension_schema_ids {
            meta.execute(
                "INSERT OR IGNORE INTO dimension_action_schemas(
               strategy, player_count, depth_bb, action_schema_id
             ) VALUES (?1, ?2, ?3, ?4)",
                &[
                    Value::from(dimension.strategy.as_str()),
                    Value::from(dimension.player_count),
                    Value::from(dimension.depth_bb),
                    Value::from(action_schema_id),
                ],
            )?;
        }
        Ok::<(), ToolError>(())
    })();
    finish_transaction(meta, build_result)?;
    write_idx_header(&mut idx, pack_count)?;
    bin.sync_all()?;
    idx.sync_all()?;
    Ok((pack_count, pack_count))
}

#[allow(clippy::too_many_arguments)]
fn flush_pack(
    connection: &Connection,
    schema_ids_by_key: &mut HashMap<String, u32>,
    dimension_schema_ids: &mut HashSet<u32>,
    concrete_line_id: u32,
    rows: &[RangeRow],
    bin: &mut File,
    idx: &mut File,
    bin_offset: &mut u32,
) -> Result<(), ToolError> {
    let encoded = encode_concrete_line_pack(rows)?;
    let schema_key = to_hex(&encoded.action_blob);
    let action_schema_id = get_or_insert_action_schema(
        connection,
        schema_ids_by_key,
        &schema_key,
        encoded.action_count,
        &encoded.action_blob,
    )?;
    dimension_schema_ids.insert(action_schema_id);

    let byte_length = u32::try_from(encoded.payload.len())
        .map_err(|_| ToolError::build("Range pack exceeds u32 byte length"))?;
    let checksum = crc32c(&encoded.payload);
    bin.seek(SeekFrom::Start(*bin_offset as u64))?;
    bin.write_all(&encoded.payload)?;
    idx.seek(SeekFrom::End(0))?;
    write_idx_record(
        idx,
        concrete_line_id,
        action_schema_id,
        encoded.hand_count,
        *bin_offset,
        byte_length,
        checksum,
    )?;
    *bin_offset = bin_offset
        .checked_add(byte_length)
        .ok_or_else(|| ToolError::build(".bin offset exceeds u32 format limit"))?;
    Ok(())
}

struct EncodedPack {
    action_blob: Vec<u8>,
    action_count: u32,
    hand_count: u16,
    payload: Vec<u8>,
}

fn encode_concrete_line_pack(rows: &[RangeRow]) -> Result<EncodedPack, ToolError> {
    if rows.is_empty() {
        return Err(ToolError::build("Cannot encode an empty concrete line"));
    }
    let mut action_set = HashSet::new();
    let mut hand_set = BTreeSet::new();
    let mut normalized_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let hand_id = get_hand_id(&row.hole_cards)?;
        let action_type = normalize_action_type(&row.action_name)?;
        let action_key = ActionKey::new(action_type, row.action_size, row.amount_bb);
        action_set.insert(action_key);
        hand_set.insert(hand_id);
        normalized_rows.push((hand_id, action_key, row.frequency, row.hand_ev));
    }

    let mut actions: Vec<_> = action_set.into_iter().collect();
    actions.sort_by(|left, right| {
        left.action_type
            .cmp(&right.action_type)
            .then(left.action_size().total_cmp(&right.action_size()))
            .then(left.amount_bb().total_cmp(&right.amount_bb()))
    });
    if actions.is_empty() || actions.len() > 32 {
        return Err(ToolError::build(format!(
            "V1 range pack requires 1..=32 actions, got {}",
            actions.len()
        )));
    }
    let hand_ids: Vec<u8> = hand_set.into_iter().collect();
    let hand_count =
        u16::try_from(hand_ids.len()).map_err(|_| ToolError::build("Hand count exceeds u16"))?;
    let action_index: HashMap<_, _> = actions
        .iter()
        .copied()
        .enumerate()
        .map(|(index, action)| (action, index))
        .collect();
    let hand_index: HashMap<_, _> = hand_ids
        .iter()
        .copied()
        .enumerate()
        .map(|(index, hand_id)| (hand_id, index))
        .collect();
    let mut masks = vec![0u32; hand_ids.len()];
    let mut values = vec![vec![(0f32, f32::NAN); actions.len()]; hand_ids.len()];
    for (hand_id, action_key, frequency, hand_ev) in normalized_rows {
        let hand_position = hand_index[&hand_id];
        let action_position = action_index[&action_key];
        masks[hand_position] |= 1u32 << action_position;
        values[hand_position][action_position] = (
            frequency as f32,
            hand_ev.map_or(f32::NAN, |value| value as f32),
        );
    }

    let mut action_blob = Vec::with_capacity(actions.len() * 9);
    for action in &actions {
        action_blob.push(action.action_type);
        action_blob.extend_from_slice(&(action.action_size() as f32).to_le_bytes());
        action_blob.extend_from_slice(&(action.amount_bb() as f32).to_le_bytes());
    }

    let mut payload = Vec::with_capacity(hand_ids.len() * (5 + actions.len().saturating_mul(8)));
    payload.extend_from_slice(&hand_ids);
    for mask in masks {
        payload.extend_from_slice(&mask.to_le_bytes());
    }
    for row in values {
        for (frequency, hand_ev) in row {
            payload.extend_from_slice(&frequency.to_le_bytes());
            payload.extend_from_slice(&hand_ev.to_le_bytes());
        }
    }
    Ok(EncodedPack {
        action_blob,
        action_count: actions.len() as u32,
        hand_count,
        payload,
    })
}

fn get_or_insert_action_schema(
    connection: &Connection,
    schema_ids_by_key: &mut HashMap<String, u32>,
    schema_key: &str,
    action_count: u32,
    action_blob: &[u8],
) -> Result<u32, ToolError> {
    if let Some(id) = schema_ids_by_key.get(schema_key) {
        return Ok(*id);
    }
    let mut select = connection.prepare("SELECT id FROM action_schemas WHERE schema_key = ?1")?;
    select.start(&[Value::from(schema_key)])?;
    if select.step_row()? {
        let id = select.column_u32(0)?;
        schema_ids_by_key.insert(schema_key.to_owned(), id);
        return Ok(id);
    }
    connection.execute(
        "INSERT INTO action_schemas(action_count, action_blob, checksum, schema_key)
         VALUES (?1, ?2, ?3, ?4)",
        &[
            Value::from(action_count),
            Value::Blob(action_blob.to_vec()),
            Value::from(i64::from(crc32c(action_blob))),
            Value::from(schema_key),
        ],
    )?;
    let id = u32::try_from(connection.last_insert_rowid())
        .map_err(|_| ToolError::build("Action schema id exceeds u32"))?;
    schema_ids_by_key.insert(schema_key.to_owned(), id);
    Ok(id)
}

fn normalize_action_type(value: &str) -> Result<u8, ToolError> {
    let normalized: String = value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| *character != '-' && *character != '_')
        .collect();
    match normalized.as_str() {
        "fold" => Ok(0),
        "call" => Ok(1),
        "check" => Ok(2),
        "bet" => Ok(3),
        "raise" => Ok(4),
        "allin" => Ok(5),
        _ => Err(ToolError::build(format!("Unknown action name: {value}"))),
    }
}

fn remove_if_exists(path: &Path) -> Result<(), ToolError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ToolError::from(error)),
    }
}

fn create_new_file(path: &Path) -> Result<File, ToolError> {
    OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .map_err(ToolError::from)
}

fn write_bin_header(file: &mut File) -> Result<(), ToolError> {
    let mut header = [0u8; PFSP_HEADER_SIZE];
    header[0..4].copy_from_slice(b"PFSP");
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[6] = 1;
    header[7] = 1;
    header[8] = 1;
    header[9] = 0;
    header[10..12].copy_from_slice(&(PFSP_HEADER_SIZE as u16).to_le_bytes());
    file.write_all(&header)?;
    Ok(())
}

fn write_idx_header(file: &mut File, record_count: u32) -> Result<(), ToolError> {
    let mut header = [0u8; IDX_HEADER_SIZE];
    header[0..4].copy_from_slice(b"PFXI");
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[8..12].copy_from_slice(&record_count.to_le_bytes());
    header[12..14].copy_from_slice(&(IDX_HEADER_SIZE as u16).to_le_bytes());
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_idx_record(
    file: &mut File,
    concrete_line_id: u32,
    action_schema_id: u32,
    hand_count: u16,
    offset: u32,
    byte_length: u32,
    checksum: u32,
) -> Result<(), ToolError> {
    let mut record = [0u8; IDX_RECORD_SIZE];
    record[0..4].copy_from_slice(&concrete_line_id.to_le_bytes());
    record[4..8].copy_from_slice(&action_schema_id.to_le_bytes());
    record[8..10].copy_from_slice(&hand_count.to_le_bytes());
    record[10..14].copy_from_slice(&offset.to_le_bytes());
    record[14..18].copy_from_slice(&byte_length.to_le_bytes());
    record[18..22].copy_from_slice(&checksum.to_le_bytes());
    file.write_all(&record)?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, ToolError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256State::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(to_hex(&hasher.finalize()))
}

struct Sha256State {
    state: [u32; 8],
    buffer: Vec<u8>,
    length_bytes: u64,
}

impl Sha256State {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: Vec::with_capacity(64),
            length_bytes: 0,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.length_bytes += bytes.len() as u64;
        if !self.buffer.is_empty() {
            let needed = 64 - self.buffer.len();
            let take = needed.min(bytes.len());
            self.buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.buffer.len() == 64 {
                let block: [u8; 64] = self.buffer.as_slice().try_into().expect("64-byte block");
                self.compress(&block);
                self.buffer.clear();
            }
        }
        while bytes.len() >= 64 {
            let block: &[u8; 64] = bytes[..64].try_into().expect("64-byte block");
            self.compress(block);
            bytes = &bytes[64..];
        }
        self.buffer.extend_from_slice(bytes);
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_length = self.length_bytes * 8;
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_length.to_be_bytes());
        let blocks = std::mem::take(&mut self.buffer);
        for block in blocks.chunks_exact(64) {
            self.compress(block.try_into().expect("64-byte block"));
        }
        let mut digest = [0u8; 32];
        for (index, word) in self.state.into_iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut words = [0u32; 64];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes(chunk.try_into().expect("4-byte word"));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(sum1)
                .wrapping_add(choose)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
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

fn utc_now_iso8601() -> Result<String, ToolError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ToolError::build(error.to_string()))?
        .as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}
