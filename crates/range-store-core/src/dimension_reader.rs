use std::io;
use std::path::Path;

use crate::bin_reader::BinReader;
use crate::crc32c::assert_crc32c;
use crate::idx_reader::IdxReader;
use crate::pack_codec::{action_count_from_pack, decode_pack_for_hand, pack_byte_length};
use crate::types::{IdxRecord, PackDecodeResult};

/// Pure Rust equivalent of the current N-API `DimensionHandle`.
///
/// It owns one mmap-backed `.idx` reader and one mmap-backed `.bin` reader for
/// a single `(strategy, player_count, depth_bb)` dimension.
#[derive(Debug)]
pub struct DimensionReader {
    idx: IdxReader,
    bin: BinReader,
}

impl DimensionReader {
    pub fn open(idx_path: &Path, bin_path: &Path) -> io::Result<Self> {
        let idx = IdxReader::open(idx_path)?;
        let bin = BinReader::open(bin_path)?;
        Ok(Self { idx, bin })
    }

    #[inline]
    pub fn record_count(&self) -> u32 {
        self.idx.record_count()
    }

    #[inline]
    pub fn unique_action_schema_ids(&self) -> Vec<u32> {
        self.idx.unique_action_schema_ids()
    }

    pub fn query(
        &self,
        concrete_line_id: u32,
        hand_id: u8,
        verify_checksum: bool,
    ) -> io::Result<Option<PackDecodeResult>> {
        let record: IdxRecord = match self.idx.find(concrete_line_id) {
            Some(record) => record,
            None => return Ok(None),
        };

        if record.hand_count == 0 {
            return Err(invalid_data(format!(
                "Invalid .idx record for concrete_line_id {}: hand_count must be > 0",
                concrete_line_id
            )));
        }

        let pack = self
            .bin
            .read_pack(record.offset, record.byte_length)
            .map_err(|e| {
                invalid_data(format!(
                    "Invalid .bin pack range for concrete_line_id {}: {}",
                    concrete_line_id, e
                ))
            })?;

        let action_count = action_count_from_pack(record.hand_count, record.byte_length);
        let expected_len = pack_byte_length(record.hand_count, action_count);
        if expected_len != record.byte_length {
            return Err(invalid_data(format!(
                "Invalid pack length for concrete_line_id {}: byte_length {} is incompatible with hand_count {}",
                concrete_line_id, record.byte_length, record.hand_count
            )));
        }

        if action_count > 32 {
            return Err(invalid_data(format!(
                "Invalid pack action count for concrete_line_id {}: {}, expected <= 32",
                concrete_line_id, action_count
            )));
        }

        if verify_checksum {
            assert_crc32c(pack, record.checksum).map_err(|reason| {
                invalid_data(format!(
                    "{}; concrete_line_id {}, expected_checksum {}",
                    reason, concrete_line_id, record.checksum
                ))
            })?;
        }

        let cells = decode_pack_for_hand(pack, record.hand_count, action_count, hand_id);
        if cells.is_empty() {
            return Ok(None);
        }

        Ok(Some(PackDecodeResult {
            action_schema_id: record.action_schema_id,
            cells,
        }))
    }
}

pub fn validate_hand_id(hand_id: u32) -> io::Result<u8> {
    if hand_id <= 168 {
        Ok(hand_id as u8)
    } else {
        Err(invalid_data(format!(
            "Invalid hand_id: {}, expected 0..=168",
            hand_id
        )))
    }
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn readers_are_send_sync() {
        assert_send_sync::<IdxReader>();
        assert_send_sync::<BinReader>();
        assert_send_sync::<DimensionReader>();
    }

    #[test]
    fn validate_hand_id_accepts_valid_range() {
        assert_eq!(validate_hand_id(0).unwrap(), 0);
        assert_eq!(validate_hand_id(168).unwrap(), 168);
    }

    #[test]
    fn validate_hand_id_rejects_out_of_range_values() {
        let err = validate_hand_id(169).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("Invalid hand_id: 169"));
    }
}
