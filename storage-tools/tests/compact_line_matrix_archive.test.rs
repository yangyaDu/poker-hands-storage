use std::fs;
use std::process::Command;

use poker_hands_storage_tools::proto_range_storage::line_matrix_store::{
    export_all_compact_line_matrix_archives, export_compact_line_matrix_archive,
    CompactLineMatrixArchive, CompactLineMatrixArchiveOptions, CompactLineMatrixArchivesOptions,
};
use poker_hands_storage_tools::proto_range_storage::proto::ActionType as CompactActionType;
use range_store_core::dimension::DimensionSpec;
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::sqlite::{Connection, Value};

#[test]
fn compact_archive_filters_null_ev_and_uses_action_local_compact_indexes() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("compact-archive");

    let summary = export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: out_dir.clone(),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("export compact archive");

    assert_eq!(summary.matrix_count, 2);
    assert_eq!(summary.action_value_count, 504);
    assert_eq!(
        fs::metadata(&summary.index_path)
            .expect("index metadata")
            .len(),
        48
    );

    let archive = CompactLineMatrixArchive::open(&out_dir).expect("open compact archive");
    assert_eq!(archive.dimension().strategy, "default");
    assert_eq!(archive.dimension().player_count, 6);
    assert_eq!(archive.dimension().depth_bb, 100);
    let verification = archive.verify_all().expect("verify compact archive");
    let sequential_verification = archive
        .verify_all_sequential()
        .expect("verify compact archive sequentially");
    assert_eq!(verification, sequential_verification);
    assert_eq!(verification.matrix_count, 2);
    assert_eq!(verification.action_count, 5);
    assert_eq!(verification.action_value_count, 504);
    let first = archive.read_matrix(1).expect("read first compact matrix");
    let matrix = first.matrix();
    assert_eq!(matrix.schema_version, 2);
    assert_eq!(matrix.valid_hand_bitmap.len(), 22);
    assert_eq!(matrix.valid_hand_bitmap[0], 0xff);
    assert!(!bit_is_set(&matrix.valid_hand_bitmap, 168));

    let call_index = matrix
        .actions
        .iter()
        .position(|action| action.action_type == CompactActionType::Call as i32)
        .expect("call action");
    let call = &matrix.actions[call_index];
    assert_eq!(call.action_hand_bitmap.len(), 21);
    assert_eq!(call.frequency_x10000.len(), 167);
    assert_eq!(call.ev_x10000.len(), 167);

    assert_eq!(first.action_value(call_index, 1), None);
    assert_eq!(first.action_value(call_index, 168), None);
    assert_eq!(
        first.action_value(call_index, 2),
        Some(
            poker_hands_storage_tools::proto_range_storage::line_matrix_store::HandActionValue {
                frequency_x10000: 2_500,
                ev_x10000: 0,
            }
        )
    );
    assert_eq!(
        first.action_value_by_identity(CompactActionType::Call, 0, 0, 2),
        first.action_value(call_index, 2)
    );
    assert_eq!(
        first.action_value_by_identity(CompactActionType::Allin, 0, 0, 2),
        None
    );
}

#[test]
fn compact_archive_does_not_scan_null_ev_rows() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "UPDATE range_data_default_6max_100BB
             SET action_name = 'unsupported', frequency = 2.0
             WHERE concrete_line_id = 1 AND hole_cards = 'AKs'
               AND action_name = 'call'",
        )
        .expect("make ignored NULL EV row invalid in other fields");

    let summary = export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
        source_db,
        out_dir: temp.path().join("compact-archive"),
        dimension: DimensionSpec::parse("default:6:100").expect("dimension"),
        overwrite: false,
    })
    .expect("NULL EV rows must not be scanned by Proto");

    assert_eq!(summary.matrix_count, 2);
}

#[test]
fn cli_exports_compact_default_6max_100bb_archive() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let out_dir = temp.path().join("compact-archive");

    let output = Command::new(env!("CARGO_BIN_EXE_poker-hands-storage-tools"))
        .args([
            "export-compact-line-matrix-archive",
            "--source-db",
            source_db.to_str().expect("source path"),
            "--out-dir",
            out_dir.to_str().expect("output path"),
        ])
        .output()
        .expect("run compact archive command");

    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_dir.join("manifest.json").is_file());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("Compact LineMatrix archive export complete."));
}

#[test]
fn exports_and_reports_all_discovered_compact_dimensions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let source_db = temp.path().join("range.db");
    create_source_fixture(&source_db);
    let connection = Connection::open(&source_db, false).expect("open fixture database");
    connection
        .exec(
            "CREATE TABLE concrete_lines_default_8max_200BB AS
               SELECT * FROM concrete_lines_default_6max_100BB;
             CREATE TABLE range_data_default_8max_200BB AS
               SELECT * FROM range_data_default_6max_100BB;",
        )
        .expect("clone fixture dimension");
    drop(connection);

    let out_dir = temp.path().join("all-compact-archives");
    let report = export_all_compact_line_matrix_archives(&CompactLineMatrixArchivesOptions {
        source_db: source_db.clone(),
        out_dir: out_dir.clone(),
        overwrite: false,
    })
    .expect("export all compact dimensions");

    assert_eq!(
        report.sqlite_bytes,
        fs::metadata(source_db).expect("sqlite metadata").len()
    );
    assert_eq!(report.dimensions.len(), 2);
    assert_eq!(report.dimensions[0].matrix_count, 2);
    assert_eq!(report.dimensions[1].matrix_count, 2);
    assert_eq!(report.dimensions[0].action_value_count, 504);
    assert_eq!(report.dimensions[1].action_value_count, 504);
    assert_eq!(
        report.total_bin_idx_bytes,
        report.total_data_bytes + report.total_index_bytes
    );
    assert!(report.bin_idx_to_sqlite_ratio > 0.0);
    assert!(out_dir.join("storage-comparison.json").is_file());
    assert!(out_dir
        .join("default_6max_100BB")
        .join("matrices.lmbin")
        .is_file());
    assert!(out_dir
        .join("default_8max_200BB")
        .join("matrices.lmidx")
        .is_file());
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

fn bit_is_set(bitmap: &[u8], hand_idx: usize) -> bool {
    bitmap[hand_idx / 8] & (1u8 << (hand_idx % 8)) != 0
}
