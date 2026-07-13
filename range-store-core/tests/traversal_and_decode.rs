use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use range_store_core::bin_reader::BinReader;
use range_store_core::idx_reader::IdxReader;
use range_store_core::pack_codec::decode_pack;
use range_store_core::types::{IdxRecord, IDX_HEADER_SIZE, IDX_RECORD_SIZE, PFSP_HEADER_SIZE};

fn make_test_idx(dir: &Path, name: &str, records: &[IdxRecord]) -> PathBuf {
    let path = dir.join(name);
    let mut file = File::create(&path).unwrap();

    let mut header = [0u8; IDX_HEADER_SIZE];
    header[0..4].copy_from_slice(b"PFXI");
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[8..12].copy_from_slice(&(records.len() as u32).to_le_bytes());
    header[12..14].copy_from_slice(&(IDX_HEADER_SIZE as u16).to_le_bytes());
    file.write_all(&header).unwrap();

    for record in records {
        let mut buf = [0u8; IDX_RECORD_SIZE];
        buf[0..4].copy_from_slice(&record.action_schema_id.to_le_bytes());
        buf[4..6].copy_from_slice(&record.hand_count.to_le_bytes());
        buf[6..10].copy_from_slice(&record.offset.to_le_bytes());
        buf[10..14].copy_from_slice(&record.byte_length.to_le_bytes());
        buf[14..18].copy_from_slice(&record.checksum.to_le_bytes());
        file.write_all(&buf).unwrap();
    }

    file.flush().unwrap();
    path
}

fn write_test_bin(path: &Path, extra_data: &[u8]) {
    let mut file = File::create(path).unwrap();
    let mut header = [0u8; PFSP_HEADER_SIZE];
    header[0..4].copy_from_slice(b"PFSP");
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[6] = 1;
    header[7] = 1;
    header[8] = 1;
    header[9] = 0;
    header[10..12].copy_from_slice(&(PFSP_HEADER_SIZE as u16).to_le_bytes());
    file.write_all(&header).unwrap();
    file.write_all(extra_data).unwrap();
    file.flush().unwrap();
}

fn make_test_pack(hand_ids: &[u8], action_count: u16, data: &[f32]) -> Vec<u8> {
    let hand_count = hand_ids.len();
    let total_actions = hand_count * action_count as usize;
    assert_eq!(data.len(), total_actions * 2);

    let byte_len = hand_count * (5 + action_count as usize * 8);
    let mut buf = vec![0u8; byte_len];
    let mut cursor = 0;

    for &id in hand_ids {
        buf[cursor] = id;
        cursor += 1;
    }

    let full_mask: u32 = if action_count == 0 {
        0
    } else if action_count >= 32 {
        u32::MAX
    } else {
        (1u32 << action_count) - 1
    };
    for _ in 0..hand_count {
        buf[cursor..cursor + 4].copy_from_slice(&full_mask.to_le_bytes());
        cursor += 4;
    }

    for chunk in data.chunks(action_count as usize * 2) {
        for cell in chunk.chunks(2) {
            buf[cursor..cursor + 4].copy_from_slice(&cell[0].to_le_bytes());
            cursor += 4;
            buf[cursor..cursor + 4].copy_from_slice(&cell[1].to_le_bytes());
            cursor += 4;
        }
    }

    buf
}

#[test]
fn idx_reader_record_at_and_records_iterate_in_file_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let records = vec![
        IdxRecord {
            action_schema_id: 1,
            hand_count: 2,
            offset: 16,
            byte_length: 42,
            checksum: 100,
        },
        IdxRecord {
            action_schema_id: 2,
            hand_count: 3,
            offset: 58,
            byte_length: 87,
            checksum: 200,
        },
    ];
    let path = make_test_idx(dir.path(), "test.idx", &records);
    let reader = IdxReader::open(&path).unwrap();

    assert_eq!(reader.record_at(0).unwrap().action_schema_id, 1);
    assert_eq!(reader.record_at(1).unwrap().action_schema_id, 2);
    assert!(reader.record_at(2).is_none());
    assert_eq!(
        reader
            .records()
            .map(|record| record.action_schema_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn bin_reader_reports_mapped_file_len() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("test.bin");
    let extra = vec![0x42u8; 100];
    write_test_bin(&path, &extra);

    let reader = BinReader::open(&path).unwrap();

    assert_eq!(reader.file_len(), PFSP_HEADER_SIZE + 100);
}

#[test]
fn decode_pack_includes_unset_cells() {
    let mut pack = make_test_pack(&[0, 2], 2, &[0.5, 1.0, 0.25, 2.0, 0.75, 3.0, 0.0, 4.0]);
    pack[2..6].copy_from_slice(&1u32.to_le_bytes());
    pack[6..10].copy_from_slice(&2u32.to_le_bytes());

    let decoded = decode_pack(&pack, 2, 2).unwrap();

    assert_eq!(decoded.hand_ids, vec![0, 2]);
    assert_eq!(decoded.action_masks, vec![1, 2]);
    assert_eq!(decoded.cells.len(), 4);
    assert!(decoded.cells[0].exists);
    assert_eq!(decoded.cells[0].hand_id, 0);
    assert_eq!(decoded.cells[0].action_id, 0);
    assert!(!decoded.cells[1].exists);
    assert_eq!(decoded.cells[2].hand_id, 2);
    assert!(!decoded.cells[2].exists);
    assert!(decoded.cells[3].exists);
    assert_eq!(decoded.cells[3].hand_ev, Some(4.0));
}

#[test]
fn decode_pack_rejects_invalid_length() {
    let err = decode_pack(&[0u8; 3], 1, 1).unwrap_err();
    assert!(err.contains("Invalid pack length"));
}
