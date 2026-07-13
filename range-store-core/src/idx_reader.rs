//! .idx file reader - mmap + direct implicit-index lookup.
//!
//! The .idx file layout:
//!   [0..16)   header  (magic PFXI, version, recordCount, headerSize)
//!   [16..]    records (18 bytes each, where record N maps to concreteLineId N + 1)

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::Path;

use memmap2::Mmap;

use crate::types::{IdxHeader, IdxRecord, IDX_HEADER_SIZE, IDX_MAGIC, IDX_RECORD_SIZE};

/// Owned mmap of an .idx file, with validated header and record count.
#[derive(Debug)]
pub struct IdxReader {
    _file: File,
    mmap: Mmap,
    record_count: u32,
}

impl IdxReader {
    /// Open and mmap the .idx file at `path`, validating the header.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < IDX_HEADER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(".idx file too small: {} bytes", file_len),
            ));
        }

        // SAFETY: the file is read-only and kept alive for the mapping lifetime.
        let mmap = unsafe { Mmap::map(&file)? };
        let header = Self::parse_header(&mmap)?;
        let expected_len =
            IDX_HEADER_SIZE as u64 + header.record_count as u64 * IDX_RECORD_SIZE as u64;
        if file_len != expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    ".idx file length mismatch: {} bytes, expected {} bytes",
                    file_len, expected_len
                ),
            ));
        }

        Ok(Self {
            _file: file,
            mmap,
            record_count: header.record_count,
        })
    }

    /// Number of records in the .idx file.
    #[inline]
    pub fn record_count(&self) -> u32 {
        self.record_count
    }

    /// Whether this .idx has one or more implicit dense records.
    #[inline]
    pub fn has_dense_index_layout(&self) -> bool {
        self.record_count > 0
    }

    /// The first concrete line id, which is always one for non-empty indexes.
    #[inline]
    pub fn first_concrete_line_id(&self) -> Option<u32> {
        (self.record_count > 0).then_some(1)
    }

    /// The last concrete line id, derived from the record count.
    #[inline]
    pub fn last_concrete_line_id(&self) -> Option<u32> {
        (self.record_count > 0).then_some(self.record_count)
    }

    /// Return the record at `index` in on-disk order.
    pub fn record_at(&self, index: u32) -> Option<IdxRecord> {
        if index >= self.record_count {
            return None;
        }
        let records_base = &self.mmap[IDX_HEADER_SIZE..];
        let offset = index as usize * IDX_RECORD_SIZE;
        Some(decode_idx_record_at(records_base, offset))
    }

    /// Iterate records in on-disk order.
    pub fn records(&self) -> impl Iterator<Item = IdxRecord> + '_ {
        (0..self.record_count).map(|index| {
            self.record_at(index)
                .expect("record index comes from record_count")
        })
    }

    /// Scan all .idx records and collect unique `action_schema_id` values.
    pub fn unique_action_schema_ids(&self) -> Vec<u32> {
        let count = self.record_count as usize;
        if count == 0 {
            return Vec::new();
        }
        let records = &self.mmap[IDX_HEADER_SIZE..];
        let mut seen = HashSet::with_capacity((count / 16).max(16));
        for index in 0..count {
            let offset = index * IDX_RECORD_SIZE;
            seen.insert(u32_from_le(&records[offset..offset + 4]));
        }
        let mut ids: Vec<u32> = seen.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// Find `concrete_line_id` through its one-based implicit array index.
    pub fn find(&self, concrete_line_id: u32) -> Option<IdxRecord> {
        let index = concrete_line_id.checked_sub(1)?;
        self.record_at(index)
    }

    fn parse_header(mmap: &[u8]) -> io::Result<IdxHeader> {
        if mmap[0..4] != IDX_MAGIC[..] {
            let magic_str = String::from_utf8_lossy(&mmap[0..4]);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid .idx magic: {}, expected PFXI", magic_str),
            ));
        }

        let version = u16::from_le_bytes([mmap[4], mmap[5]]);
        if version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported .idx version: {}", version),
            ));
        }

        let record_count = u32::from_le_bytes([mmap[8], mmap[9], mmap[10], mmap[11]]);
        let header_size = u16::from_le_bytes([mmap[12], mmap[13]]);
        if header_size as usize != IDX_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported .idx header size: {}", header_size),
            ));
        }

        Ok(IdxHeader { record_count })
    }
}

/// Decode a single implicit-id record from `data[offset..offset + 18]`.
#[inline]
fn decode_idx_record_at(data: &[u8], offset: usize) -> IdxRecord {
    let bytes = &data[offset..offset + IDX_RECORD_SIZE];
    IdxRecord {
        action_schema_id: u32_from_le(&bytes[0..4]),
        hand_count: u16::from_le_bytes([bytes[4], bytes[5]]),
        offset: u32_from_le(&bytes[6..10]),
        byte_length: u32_from_le(&bytes[10..14]),
        checksum: u32_from_le(&bytes[14..18]),
    }
}

#[inline(always)]
fn u32_from_le(slice: &[u8]) -> u32 {
    u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_test_idx(
        dir: &std::path::Path,
        name: &str,
        records: &[IdxRecord],
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        let mut header = [0u8; IDX_HEADER_SIZE];
        header[0..4].copy_from_slice(b"PFXI");
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[8..12].copy_from_slice(&(records.len() as u32).to_le_bytes());
        header[12..14].copy_from_slice(&(IDX_HEADER_SIZE as u16).to_le_bytes());
        file.write_all(&header).unwrap();

        for record in records {
            let mut bytes = [0u8; IDX_RECORD_SIZE];
            bytes[0..4].copy_from_slice(&record.action_schema_id.to_le_bytes());
            bytes[4..6].copy_from_slice(&record.hand_count.to_le_bytes());
            bytes[6..10].copy_from_slice(&record.offset.to_le_bytes());
            bytes[10..14].copy_from_slice(&record.byte_length.to_le_bytes());
            bytes[14..18].copy_from_slice(&record.checksum.to_le_bytes());
            file.write_all(&bytes).unwrap();
        }
        file.flush().unwrap();
        path
    }

    fn record(action_schema_id: u32, offset: u32) -> IdxRecord {
        IdxRecord {
            action_schema_id,
            hand_count: 100,
            offset,
            byte_length: 5000,
            checksum: 0xDEADBEEF,
        }
    }

    #[test]
    fn open_empty_idx_has_no_implicit_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = make_test_idx(dir.path(), "test.idx", &[]);
        let reader = IdxReader::open(&path).unwrap();
        assert_eq!(reader.record_count(), 0);
        assert!(!reader.has_dense_index_layout());
        assert_eq!(reader.first_concrete_line_id(), None);
        assert_eq!(reader.last_concrete_line_id(), None);
        assert!(reader.find(1).is_none());
    }

    #[test]
    fn implicit_index_lookup_uses_one_based_line_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let records = vec![record(1, 100), record(2, 200), record(3, 300)];
        let path = make_test_idx(dir.path(), "test.idx", &records);
        let reader = IdxReader::open(&path).unwrap();
        assert_eq!(reader.record_count(), 3);
        assert!(reader.has_dense_index_layout());
        assert_eq!(reader.first_concrete_line_id(), Some(1));
        assert_eq!(reader.last_concrete_line_id(), Some(3));
        assert_eq!(reader.find(2).unwrap().action_schema_id, 2);
        assert_eq!(reader.find(3).unwrap().offset, 300);
        assert!(reader.find(0).is_none());
        assert!(reader.find(4).is_none());
    }

    #[test]
    fn record_iteration_preserves_file_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = make_test_idx(dir.path(), "test.idx", &[record(7, 100), record(2, 200)]);
        let reader = IdxReader::open(&path).unwrap();
        assert_eq!(reader.record_at(0).unwrap().action_schema_id, 7);
        assert_eq!(reader.record_at(1).unwrap().action_schema_id, 2);
        assert_eq!(
            reader
                .records()
                .map(|record| record.action_schema_id)
                .collect::<Vec<_>>(),
            vec![7, 2]
        );
    }

    #[test]
    fn invalid_magic_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.idx");
        let mut header = [0u8; IDX_HEADER_SIZE];
        header[0..4].copy_from_slice(b"XXXX");
        std::fs::write(&path, header).unwrap();
        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Invalid .idx magic"));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.idx");
        let mut header = [0u8; IDX_HEADER_SIZE];
        header[0..4].copy_from_slice(b"PFXI");
        header[4..6].copy_from_slice(&99u16.to_le_bytes());
        std::fs::write(&path, header).unwrap();
        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Unsupported .idx version"));
    }
    #[test]
    fn unexpected_trailing_bytes_are_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = make_test_idx(dir.path(), "trailing.idx", &[record(1, 100)]);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(&[0]).unwrap();
        file.flush().unwrap();

        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("file length mismatch"));
    }
}
