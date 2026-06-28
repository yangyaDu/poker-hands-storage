use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use poker_hands_storage_service::storage::sqlite::{Connection, Value};
use range_store_core::crc32c::crc32c;
use range_store_core::types::{IDX_HEADER_SIZE, IDX_RECORD_SIZE, PFSP_HEADER_SIZE};

/// Build a minimal test store with one dimension (default:6:100) containing
/// two concrete lines and a few hands, using only `range-store-core` binary
/// format primitives. Does NOT depend on `range_store_builder`.
pub fn build_api_test_store(root: &Path) -> PathBuf {
    let output_path = root.join("output");
    fs::create_dir_all(&output_path).unwrap();

    let meta = Connection::open(&output_path.join("meta.db"), false).unwrap();
    meta.exec(
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
         );
         CREATE TABLE \"concrete_lines_default_6max_100BB\" (
           concrete_line_id INTEGER PRIMARY KEY,
           abstract_line TEXT NOT NULL,
           concrete_line TEXT NOT NULL,
           UNIQUE(abstract_line, concrete_line)
         );
         CREATE TABLE \"drill_scenario_lines_default\" (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           drill_name TEXT NOT NULL,
           abstract_line TEXT NOT NULL,
           player_count INTEGER NOT NULL,
           drill_depth INTEGER NOT NULL DEFAULT 100,
           UNIQUE(drill_name, player_count, drill_depth, abstract_line)
         );
         INSERT INTO \"concrete_lines_default_6max_100BB\"
           VALUES (1, 'F-F-F', 'F-F-F');
         INSERT INTO \"drill_scenario_lines_default\"(drill_name, abstract_line, player_count, drill_depth)
           VALUES ('rfi', 'F-F-F', 6, 100);",
    )
    .unwrap();

    // Build action schema: fold(0,0,0) + raise(4, 2.5f32, 2.5f32)
    let mut action_blob = Vec::with_capacity(18);
    action_blob.push(0u8); // fold
    action_blob.extend_from_slice(&0f32.to_le_bytes()); // action_size
    action_blob.extend_from_slice(&0f32.to_le_bytes()); // amount_bb
    action_blob.push(4u8); // raise
    action_blob.extend_from_slice(&2.5f32.to_le_bytes());
    action_blob.extend_from_slice(&2.5f32.to_le_bytes());

    let schema_checksum = crc32c(&action_blob);
    let schema_key = action_blob
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    meta.execute(
        "INSERT INTO action_schemas(action_count, action_blob, checksum, schema_key) VALUES (?1, ?2, ?3, ?4)",
        &[
            Value::from(2u32),
            Value::Blob(action_blob),
            Value::from(i64::from(schema_checksum)),
            Value::from(schema_key.as_str()),
        ],
    ).unwrap();

    meta.execute(
        "INSERT INTO dimension_action_schemas(strategy, player_count, depth_bb, action_schema_id) VALUES (?1, ?2, ?3, ?4)",
        &[
            Value::from("default"),
            Value::from(6u32),
            Value::from(100u32),
            Value::from(1u32),
        ],
    ).unwrap();

    // Also add '72o' hand with fold-only action (0 frequency — used by router test
    // to verify hands-by-actions frequency threshold behaviour).
    // For simplicity we put both hands (AA and 72o) in pack 1.

    // Build PFSP .bin file with one pack for concrete_line_id=1
    // Pack layout: [hand_ids] [masks_u32_le] [freq_ev_f32_le...]
    //
    // Hands: AA => hand_id=0, 72o => hand_id=162 (row=12,col=5 in 13x13)
    // Actions: fold=idx0, raise=idx1
    // AA: mask = fold|raise = 0b11, fold_freq=0.25, fold_ev=NaN, raise_freq=0.75, raise_ev=1.0
    // 72o: mask = fold = 0b01, fold_freq=0.0, fold_ev=NaN, raise_freq=0, raise_ev=NaN

    let hand_ids: Vec<u8> = vec![0, 162]; // AA=0, 72o=162
    let masks: Vec<u32> = vec![0b11, 0b01]; // AA has both, 72o has only fold
    let values_aa = [0.25f32, f32::NAN, 0.75f32, 1.0f32]; // fold_freq, fold_ev, raise_freq, raise_ev
    let values_72o = [0.0f32, f32::NAN, 0.0f32, f32::NAN]; // fold_freq, fold_ev, raise_freq(unused), raise_ev(unused)

    let mut payload = Vec::new();
    for &h in &hand_ids {
        payload.push(h);
    }
    for &m in &masks {
        payload.extend_from_slice(&m.to_le_bytes());
    }
    for v in &values_aa {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    for v in &values_72o {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    let pack_checksum = crc32c(&payload);

    // Write .bin file
    let bin_path = output_path.join("ranges_default_6max_100BB.bin");
    let mut bin = fs::File::create(&bin_path).unwrap();
    let mut bin_header = [0u8; PFSP_HEADER_SIZE];
    bin_header[0..4].copy_from_slice(b"PFSP");
    bin_header[4..6].copy_from_slice(&1u16.to_le_bytes());
    bin_header[6] = 1;
    bin_header[7] = 1;
    bin_header[8] = 1;
    bin_header[9] = 0;
    bin_header[10..12].copy_from_slice(&(PFSP_HEADER_SIZE as u16).to_le_bytes());
    bin.write_all(&bin_header).unwrap();
    bin.write_all(&payload).unwrap();
    bin.sync_all().unwrap();

    // Write .idx file: header + 1 record
    let idx_path = output_path.join("ranges_default_6max_100BB.idx");
    let mut idx = fs::File::create(&idx_path).unwrap();
    let mut idx_header = [0u8; IDX_HEADER_SIZE];
    idx_header[0..4].copy_from_slice(b"PFXI");
    idx_header[4..6].copy_from_slice(&1u16.to_le_bytes());
    idx_header[8..12].copy_from_slice(&1u32.to_le_bytes()); // 1 record
    idx_header[12..14].copy_from_slice(&(IDX_HEADER_SIZE as u16).to_le_bytes());
    idx.write_all(&idx_header).unwrap();

    // IDX record: concrete_line_id(u32) + action_schema_id(u32) + hand_count(u16) + bin_offset(u32) + byte_length(u32) + checksum(u32) = 22 bytes
    let byte_length = payload.len() as u32;
    let bin_offset = PFSP_HEADER_SIZE as u32;
    let mut record = [0u8; IDX_RECORD_SIZE];
    record[0..4].copy_from_slice(&1u32.to_le_bytes()); // concrete_line_id
    record[4..8].copy_from_slice(&1u32.to_le_bytes()); // action_schema_id
    record[8..10].copy_from_slice(&(hand_ids.len() as u16).to_le_bytes());
    record[10..14].copy_from_slice(&bin_offset.to_le_bytes());
    record[14..18].copy_from_slice(&byte_length.to_le_bytes());
    record[18..22].copy_from_slice(&pack_checksum.to_le_bytes());
    idx.write_all(&record).unwrap();
    idx.sync_all().unwrap();

    // Write manifest.json
    let manifest = serde_json::json!({
        "format": "PFSP",
        "version": 1,
        "sourceDbChecksum": "fixture",
        "builtAt": "2026-06-28T00:00:00Z",
        "dimensions": [{
            "strategy": "default",
            "playerCount": 6,
            "depthBb": 100,
            "concreteLineCount": 1,
            "packCount": 1,
            "status": "success",
            "binFile": "ranges_default_6max_100BB.bin",
            "idxFile": "ranges_default_6max_100BB.idx",
            "binFileSizeBytes": fs::metadata(&bin_path).unwrap().len(),
            "idxFileSizeBytes": fs::metadata(&idx_path).unwrap().len()
        }],
        "files": [
            "meta.db",
            "ranges_default_6max_100BB.bin",
            "ranges_default_6max_100BB.idx"
        ]
    });
    fs::write(
        output_path.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    output_path
}

pub fn build_empty_store(root: &Path) -> PathBuf {
    let output_path = root.join("output-empty");
    fs::create_dir(&output_path).unwrap();
    fs::write(
        output_path.join("manifest.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "format": "PFSP",
            "version": 1,
            "sourceDbChecksum": "fixture",
            "builtAt": "2026-06-28T00:00:00Z",
            "dimensions": [],
            "files": ["meta.db"]
        }))
        .unwrap(),
    )
    .unwrap();
    let meta = Connection::open(&output_path.join("meta.db"), false).unwrap();
    meta.exec(
        "CREATE TABLE action_schemas (
           id INTEGER PRIMARY KEY,
           action_count INTEGER NOT NULL,
           action_blob BLOB NOT NULL
         );
         CREATE TABLE dimension_action_schemas (
           strategy TEXT NOT NULL,
           player_count INTEGER NOT NULL,
           depth_bb INTEGER NOT NULL,
           action_schema_id INTEGER NOT NULL
         );",
    )
    .unwrap();
    output_path
}
