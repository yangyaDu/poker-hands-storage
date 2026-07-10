use std::fs;
use std::process::Command;

use poker_hands_storage_tools::line_matrix_archive::{
    export_line_matrix_archive, LineMatrixArchive, LineMatrixArchiveOptions,
};
use poker_hands_storage_tools::line_matrix_export::proto::{
    ActionColumn, ActionType, HandEncoding, LineMatrix,
};
use poker_hands_storage_tools::line_matrix_export::{
    export_line_matrix, ConcreteLineSelector, ExportLineMatrixOptions,
};
use prost::Message;
use range_store_core::dimension::DimensionSpec;
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::sqlite::{Connection, Value};

#[test]
fn exports_sparse_action_columns_and_null_ev_from_id_and_text_selectors() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);

    let dimension = DimensionSpec::parse("default:6:100").expect("dimension");
    let id_out = temp.path().join("by-id");
    let summary = export_line_matrix(&ExportLineMatrixOptions {
        source_db: source_db.clone(),
        out_dir: id_out,
        dimension: dimension.clone(),
        selector: ConcreteLineSelector::Id(1),
        gto_data_version: "fixture-001".to_owned(),
        overwrite: false,
    })
    .expect("export by id");

    assert_eq!(summary.concrete_line_id, 1);
    assert_eq!(summary.action_count, 3);
    assert_eq!(summary.source_row_count, 503);
    assert_eq!(summary.null_ev_count, 1);
    assert_eq!(summary.hands_with_actions, 168);
    assert_eq!(summary.hands_without_actions, 1);
    assert_eq!(summary.frequency_sum_mismatch_hand_count, 1);
    assert_eq!(summary.max_frequency_error_x10000, 200);
    assert!(summary.protobuf_bytes > 0);
    assert!(summary.debug_json_path.is_file());
    assert!(summary.verify_json_path.is_file());

    let protobuf = fs::read(&summary.protobuf_path).expect("read protobuf");
    let matrix = LineMatrix::decode(protobuf.as_slice()).expect("decode protobuf");
    assert_eq!(matrix.schema_version, 1);
    assert_eq!(matrix.gto_data_version, "fixture-001");
    assert_eq!(matrix.hand_encoding, HandEncoding::HandEncoding169 as i32);
    assert_eq!(matrix.invalid_hand_bitmap, vec![0; 22]);

    let fold = action(&matrix, ActionType::Fold);
    let call = action(&matrix, ActionType::Call);
    let raise = action(&matrix, ActionType::Raise);
    assert_eq!(fold.action_size_x10000, 0);
    assert_eq!(fold.amount_centi_bb, 0);
    assert_eq!(raise.action_size_x10000, 400_000);
    assert_eq!(raise.amount_centi_bb, 200);
    assert_eq!(raise.frequency_x10000.len(), 169);
    assert_eq!(raise.ev_x10000.len(), 169);
    assert_eq!(raise.action_hand_bitmap.len(), 22);
    assert_eq!(raise.ev_null_bitmap.len(), 22);

    // AA (hand_idx=0) has no raise, while every other hand does.
    assert!(!bit_is_set(&raise.action_hand_bitmap, 0));
    assert_eq!(raise.frequency_x10000[0], 0);
    assert_eq!(raise.ev_x10000[0], 0);
    assert!(bit_is_set(&raise.action_hand_bitmap, 1));

    // AKs (hand_idx=1) has CALL with NULL EV.
    assert!(bit_is_set(&call.action_hand_bitmap, 1));
    assert!(bit_is_set(&call.ev_null_bitmap, 1));
    assert_eq!(call.ev_x10000[1], 0);

    // AQs (hand_idx=2) has a real EV of zero, distinct from NULL.
    assert!(bit_is_set(&call.action_hand_bitmap, 2));
    assert!(!bit_is_set(&call.ev_null_bitmap, 2));
    assert_eq!(call.ev_x10000[2], 0);

    let verify: serde_json::Value =
        serde_json::from_slice(&fs::read(summary.verify_json_path).expect("read verify"))
            .expect("parse verify");
    assert_eq!(verify["pass"], true);
    assert_eq!(verify["presentActionCellCount"], 503);
    assert_eq!(verify["nullEvCount"], 1);
    assert_eq!(verify["handsWithActions"], 168);
    assert_eq!(verify["handsWithoutActions"], 1);
    assert_eq!(verify["frequencySumMismatchHandCount"], 1);
    assert_eq!(verify["maxFrequencyErrorX10000"], 200);
    assert_eq!(
        verify["checks"]["sourceFrequencySumsWithinRoundingTolerance"],
        false
    );
    assert_eq!(verify["warnings"].as_array().expect("warnings").len(), 1);

    let text_summary = export_line_matrix(&ExportLineMatrixOptions {
        source_db,
        out_dir: temp.path().join("by-text"),
        dimension,
        selector: ConcreteLineSelector::Text {
            concrete_line: "F-F-F".to_owned(),
            abstract_line: None,
        },
        gto_data_version: "fixture-001".to_owned(),
        overwrite: false,
    })
    .expect("export by line text");
    assert_eq!(text_summary.concrete_line_id, 1);
}

#[test]
fn exports_default_6max_100bb_lines_as_a_dense_indexed_archive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("archive");

    let summary = export_line_matrix_archive(&LineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        gto_data_version: "fixture-001".to_owned(),
        overwrite: false,
    })
    .expect("export archive");

    assert_eq!(summary.matrix_count, 2);
    assert_eq!(
        summary.data_path.file_name().and_then(|name| name.to_str()),
        Some("matrices.lmbin")
    );
    assert_eq!(
        summary
            .index_path
            .file_name()
            .and_then(|name| name.to_str()),
        Some("matrices.lmidx")
    );
    assert!(summary.manifest_path.is_file());
    assert!(summary.metadata_path.is_file());
    assert_eq!(
        fs::metadata(&summary.index_path)
            .expect("index metadata")
            .len(),
        48
    );

    let archive = LineMatrixArchive::open(&out_dir).expect("open archive");
    assert_eq!(archive.matrix_count(), 2);
    let second = archive.read_matrix(2).expect("read second matrix");
    assert_eq!(second.gto_data_version, "fixture-001");
    assert_eq!(second.hand_encoding, HandEncoding::HandEncoding169 as i32);
    assert_eq!(second.actions.len(), 2);

    let metadata = Connection::open(&summary.metadata_path, true).expect("open archive metadata");
    let mut statement = metadata
        .prepare(
            "SELECT abstract_line, concrete_line FROM concrete_lines WHERE concrete_line_id = ?1",
        )
        .expect("prepare metadata query");
    statement
        .start(&[Value::from(2u32)])
        .expect("query metadata");
    assert!(statement.step_row().expect("metadata row"));
    assert_eq!(statement.column_text(0).expect("abstract line"), "R-F");
    assert_eq!(statement.column_text(1).expect("concrete line"), "R2-F");
}

#[test]
fn cli_exports_default_6max_100bb_archive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("archive");

    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "export-line-matrix-archive",
            "--source-db",
            source_db.to_str().expect("source path"),
            "--out-dir",
            out_dir.to_str().expect("output path"),
            "--gto-data-version",
            "fixture-001",
        ])
        .output()
        .expect("run archive command");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_dir.join("manifest.json").is_file());
    assert!(String::from_utf8_lossy(&output.stdout).contains("LineMatrix archive export complete."));
}

fn create_source_fixture(path: &std::path::Path) {
    let connection = Connection::open(path, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_6max_100BB(
               id INTEGER PRIMARY KEY,
               abstract_line TEXT NOT NULL,
               concrete_line TEXT NOT NULL
             );
             CREATE TABLE range_data_default_6max_100BB(
               concrete_line_id INTEGER NOT NULL,
               hole_cards TEXT NOT NULL,
               action_name TEXT NOT NULL,
               action_size REAL NOT NULL,
               amount_bb REAL NOT NULL,
               frequency REAL NOT NULL,
               hand_ev REAL
             );",
        )
        .expect("create fixture tables");
    connection
        .execute(
            "INSERT INTO concrete_lines_default_6max_100BB(id, abstract_line, concrete_line)
             VALUES (?1, ?2, ?3)",
            &[
                Value::from(1u32),
                Value::from("F-F-F"),
                Value::from("F-F-F"),
            ],
        )
        .expect("insert line");
    connection
        .execute(
            "INSERT INTO concrete_lines_default_6max_100BB(id, abstract_line, concrete_line)
             VALUES (?1, ?2, ?3)",
            &[Value::from(2u32), Value::from("R-F"), Value::from("R2-F")],
        )
        .expect("insert second line");

    connection.exec("BEGIN").expect("begin fixture transaction");
    let mut insert = connection
        .prepare(
            "INSERT INTO range_data_default_6max_100BB(
               concrete_line_id, hole_cards, action_name, action_size,
               amount_bb, frequency, hand_ev
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .expect("prepare range insert");
    for hand_idx in 0u8..=168 {
        // 22 has no rows at all on this line; this is distinct from an invalid hand.
        if hand_idx == 168 {
            continue;
        }
        let hand = hand_code_from_id(hand_idx);
        let fold_frequency = if hand_idx == 0 {
            0.5
        } else if hand_idx == 167 {
            0.24
        } else {
            0.25
        };
        let call_frequency = if hand_idx == 0 {
            0.5
        } else if hand_idx == 167 {
            0.24
        } else {
            0.25
        };
        insert_row(
            &mut insert,
            &hand,
            "fold",
            0.0,
            0.0,
            fold_frequency,
            Value::from(-0.25f64),
        );
        let call_ev = if hand_idx == 1 {
            Value::Null
        } else if hand_idx == 2 {
            Value::from(0.0f64)
        } else {
            Value::from(0.5f64)
        };
        insert_row(
            &mut insert,
            &hand,
            "call",
            0.0,
            0.0,
            call_frequency,
            call_ev,
        );
        if hand_idx != 0 {
            insert_row(
                &mut insert,
                &hand,
                "raise",
                40.0,
                2.0,
                0.5,
                Value::from(1.25f64),
            );
        }
    }
    insert_row_for_line(
        &mut insert,
        2,
        "AA",
        "fold",
        0.0,
        0.0,
        0.25,
        Value::from(-0.5f64),
    );
    insert_row_for_line(
        &mut insert,
        2,
        "AA",
        "raise",
        40.0,
        2.0,
        0.75,
        Value::from(1.5f64),
    );
    drop(insert);
    connection
        .exec("COMMIT")
        .expect("commit fixture transaction");
}

#[allow(clippy::too_many_arguments)]
fn insert_row(
    insert: &mut range_store_core::sqlite::Statement<'_>,
    hand: &str,
    action: &str,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Value,
) {
    insert_row_for_line(
        insert,
        1,
        hand,
        action,
        action_size,
        amount_bb,
        frequency,
        hand_ev,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_row_for_line(
    insert: &mut range_store_core::sqlite::Statement<'_>,
    concrete_line_id: u32,
    hand: &str,
    action: &str,
    action_size: f64,
    amount_bb: f64,
    frequency: f64,
    hand_ev: Value,
) {
    insert
        .execute(&[
            Value::from(concrete_line_id),
            Value::from(hand),
            Value::from(action),
            Value::from(action_size),
            Value::from(amount_bb),
            Value::from(frequency),
            hand_ev,
        ])
        .expect("insert range row");
}

fn action(matrix: &LineMatrix, action_type: ActionType) -> &ActionColumn {
    matrix
        .actions
        .iter()
        .find(|action| action.action_type == action_type as i32)
        .expect("action column")
}

fn bit_is_set(bitmap: &[u8], hand_idx: usize) -> bool {
    bitmap[hand_idx / 8] & (1u8 << (hand_idx % 8)) != 0
}
