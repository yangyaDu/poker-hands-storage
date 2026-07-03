//! .idx file reader — mmap + direct dense-index lookup.
//!
//! The .idx file layout:
//!   [0..16)   header  (magic PFXI, version, recordCount, headerSize)
//!   [16..]    records (22 bytes each, sorted by concreteLineId ascending)

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
    first_concrete_line_id: Option<u32>,
    last_concrete_line_id: Option<u32>,
}

impl IdxReader {
    /// Open and mmap the .idx file at `path`, validating the header.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;

        // Verify file is large enough for header
        let meta = file.metadata()?;
        let file_len = meta.len();
        if file_len < IDX_HEADER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(".idx file too small: {} bytes", file_len),
            ));
        }

        // SAFETY: the file is opened read-only, has already been checked to be
        // at least large enough for the fixed .idx header, and the `File` is
        // kept alive in `IdxReader` for the full lifetime of the mmap. After
        // mapping we validate header fields and the declared record area length
        // before any record-level access.
        //
        // This assumes generated index files are immutable while a
        // `DimensionHandle` is alive. An external process that truncates or
        // mutates the same file can still invalidate the OS mapping (for
        // example SIGBUS on Unix); callers should deploy by writing new
        // versioned directories and swapping handles instead of modifying files
        // in place.
        let mmap = unsafe { Mmap::map(&file)? };

        // Parse & validate header
        let header = Self::parse_header(&mmap)?;

        // Verify file is large enough for all records
        let expected_len =
            IDX_HEADER_SIZE as u64 + header.record_count as u64 * IDX_RECORD_SIZE as u64;
        if file_len < expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    ".idx file truncated: {} bytes, expected >= {} bytes",
                    file_len, expected_len
                ),
            ));
        }

        let metadata = validate_dense_index_layout(&mmap, header.record_count)?;

        Ok(Self {
            _file: file,
            mmap,
            record_count: header.record_count,
            first_concrete_line_id: metadata.first_concrete_line_id,
            last_concrete_line_id: metadata.last_concrete_line_id,
        })
    }

    /// Number of records in the .idx file.
    #[inline]
    pub fn record_count(&self) -> u32 {
        self.record_count
    }

    /// Whether this .idx has records and passed dense layout validation.
    #[inline]
    pub fn has_dense_index_layout(&self) -> bool {
        self.record_count > 0
    }

    #[inline]
    pub fn first_concrete_line_id(&self) -> Option<u32> {
        self.first_concrete_line_id
    }

    #[inline]
    pub fn last_concrete_line_id(&self) -> Option<u32> {
        self.last_concrete_line_id
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
    ///
    /// Used by the TS layer to prewarm only the subset of action schemas
    /// actually referenced by this dimension, instead of loading all schemas.
    pub fn unique_action_schema_ids(&self) -> Vec<u32> {
        let n = self.record_count as usize;
        if n == 0 {
            return Vec::new();
        }
        let records = &self.mmap[IDX_HEADER_SIZE..];
        let mut seen = HashSet::with_capacity((n / 16).max(16));
        for i in 0..n {
            let off = i * IDX_RECORD_SIZE;
            seen.insert(u32_from_le(&records[off + 4..off + 8]));
        }
        let mut ids: Vec<u32> = seen.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// Find `concrete_line_id`.
    ///
    /// `.idx` records are required to be dense and contiguous at open time, so
    /// lookup is a direct index calculation:
    /// `record_index = concrete_line_id - first_concrete_line_id`.
    ///
    /// Returns `None` if not found.
    pub fn find(&self, concrete_line_id: u32) -> Option<IdxRecord> {
        if self.record_count == 0 {
            return None;
        }

        self.find_by_dense_index(concrete_line_id)
    }

    fn find_by_dense_index(&self, concrete_line_id: u32) -> Option<IdxRecord> {
        let first_id = self.first_concrete_line_id?;
        let index = concrete_line_id.checked_sub(first_id)?;
        if index >= self.record_count {
            return None;
        }

        let records_base = &self.mmap[IDX_HEADER_SIZE..];
        let offset = index as usize * IDX_RECORD_SIZE;
        let record = decode_idx_record_at(records_base, offset);
        if record.concrete_line_id == concrete_line_id {
            Some(record)
        } else {
            None
        }
    }
}

struct DenseIndexMetadata {
    first_concrete_line_id: Option<u32>,
    last_concrete_line_id: Option<u32>,
}

fn validate_dense_index_layout(mmap: &[u8], record_count: u32) -> io::Result<DenseIndexMetadata> {
    if record_count == 0 {
        return Ok(DenseIndexMetadata {
            first_concrete_line_id: None,
            last_concrete_line_id: None,
        });
    }

    let records_base = &mmap[IDX_HEADER_SIZE..];
    let first_id = concrete_line_id_at(records_base, 0);
    let mut expected_id = first_id;
    let mut last_id = first_id;

    for index in 0..record_count as usize {
        let offset = index * IDX_RECORD_SIZE;
        let actual_id = concrete_line_id_at(records_base, offset);
        if actual_id != expected_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    ".idx concreteLineId must be contiguous for dense lookup: record {index} has {actual_id}, expected {expected_id}"
                ),
            ));
        }

        last_id = actual_id;
        if index + 1 < record_count as usize {
            expected_id = expected_id.checked_add(1).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    ".idx concreteLineId overflows u32 while validating dense lookup layout",
                )
            })?;
        }
    }

    Ok(DenseIndexMetadata {
        first_concrete_line_id: Some(first_id),
        last_concrete_line_id: Some(last_id),
    })
}

#[inline]
fn concrete_line_id_at(records_base: &[u8], offset: usize) -> u32 {
    u32_from_le(&records_base[offset..offset + 4])
}

/// Decode a single IdxRecord from `data[offset..offset + 22]`.
///
/// Safety: caller must ensure offset + IDX_RECORD_SIZE <= data.len().
#[inline]
fn decode_idx_record_at(data: &[u8], offset: usize) -> IdxRecord {
    let b = &data[offset..offset + IDX_RECORD_SIZE];

    // SAFETY: byte-array-to-u32 via ptr::read_unaligned is sound when bounds are checked.
    // We use from_le_bytes for clarity; the compiler optimizes it to a single load on LE archs.
    let concrete_line_id = u32_from_le(&b[0..4]);
    let action_schema_id = u32_from_le(&b[4..8]);
    let hand_count = u16::from_le_bytes([b[8], b[9]]);
    let offset = u32_from_le(&b[10..14]);
    let byte_length = u32_from_le(&b[14..18]);
    let checksum = u32_from_le(&b[18..22]);

    IdxRecord {
        concrete_line_id,
        action_schema_id,
        hand_count,
        offset,
        byte_length,
        checksum,
    }
}

#[inline(always)]
fn u32_from_le(slice: &[u8]) -> u32 {
    u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
}

impl IdxReader {
    fn parse_header(mmap: &[u8]) -> io::Result<IdxHeader> {
        // Check magic
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
        let mut f = File::create(&path).unwrap();

        let mut header = [0u8; IDX_HEADER_SIZE];
        header[0..4].copy_from_slice(b"PFXI");
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[8..12].copy_from_slice(&(records.len() as u32).to_le_bytes());
        header[12..14].copy_from_slice(&(IDX_HEADER_SIZE as u16).to_le_bytes());
        f.write_all(&header).unwrap();

        for r in records {
            let mut buf = [0u8; IDX_RECORD_SIZE];
            buf[0..4].copy_from_slice(&r.concrete_line_id.to_le_bytes());
            buf[4..8].copy_from_slice(&r.action_schema_id.to_le_bytes());
            buf[8..10].copy_from_slice(&r.hand_count.to_le_bytes());
            buf[10..14].copy_from_slice(&r.offset.to_le_bytes());
            buf[14..18].copy_from_slice(&r.byte_length.to_le_bytes());
            buf[18..22].copy_from_slice(&r.checksum.to_le_bytes());
            f.write_all(&buf).unwrap();
        }

        f.flush().unwrap();
        path
    }

    fn record(concrete_line_id: u32, action_schema_id: u32) -> IdxRecord {
        IdxRecord {
            concrete_line_id,
            action_schema_id,
            hand_count: 100,
            offset: concrete_line_id * 100,
            byte_length: 5000,
            checksum: 0xDEADBEEF,
        }
    }

    #[test]
    fn test_open_empty_idx() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = make_test_idx(dir.path(), "test.idx", &[]);
        let reader = IdxReader::open(&path).unwrap();
        assert_eq!(reader.record_count(), 0);
        assert!(!reader.has_dense_index_layout());
        assert_eq!(reader.first_concrete_line_id(), None);
        assert_eq!(reader.last_concrete_line_id(), None);
        assert!(reader.find(1).is_none());
        drop(reader);
    }

    #[test]
    fn test_dense_index_lookup() {
        let dir = tempfile::TempDir::new().unwrap();
        let records = vec![record(10, 1), record(11, 2), record(12, 3)];

        let path = make_test_idx(dir.path(), "test.idx", &records);
        let reader = IdxReader::open(&path).unwrap();
        assert_eq!(reader.record_count(), 3);
        assert!(reader.has_dense_index_layout());
        assert_eq!(reader.first_concrete_line_id(), Some(10));
        assert_eq!(reader.last_concrete_line_id(), Some(12));

        let r = reader.find(11).unwrap();
        assert_eq!(r.concrete_line_id, 11);
        assert_eq!(r.action_schema_id, 2);

        assert!(reader.find(9).is_none());
        assert!(reader.find(13).is_none());
        drop(reader);
    }

    #[test]
    fn test_sparse_index_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let records = vec![
            IdxRecord {
                concrete_line_id: 10,
                action_schema_id: 1,
                hand_count: 100,
                offset: 0,
                byte_length: 5000,
                checksum: 0xDEADBEEF,
            },
            IdxRecord {
                concrete_line_id: 20,
                action_schema_id: 2,
                hand_count: 50,
                offset: 5000,
                byte_length: 2500,
                checksum: 0xCAFEBABE,
            },
            IdxRecord {
                concrete_line_id: 30,
                action_schema_id: 3,
                hand_count: 169,
                offset: 7500,
                byte_length: 84500,
                checksum: 0x12345678,
            },
        ];

        let path = make_test_idx(dir.path(), "test.idx", &records);
        let err = IdxReader::open(&path).unwrap_err();
        assert!(err
            .to_string()
            .contains("concreteLineId must be contiguous"));
    }

    #[test]
    fn test_single_record_is_dense_candidate() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = make_test_idx(dir.path(), "test.idx", &[record(1, 7)]);
        let reader = IdxReader::open(&path).unwrap();

        assert!(reader.has_dense_index_layout());
        assert_eq!(reader.first_concrete_line_id(), Some(1));
        assert_eq!(reader.last_concrete_line_id(), Some(1));
        assert_eq!(reader.find(1).unwrap().action_schema_id, 7);
        assert!(reader.find(2).is_none());
    }

    #[test]
    fn test_internal_gap_or_duplicate_is_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let records = vec![record(10, 1), record(12, 2), record(12, 3)];

        let path = make_test_idx(dir.path(), "test.idx", &records);
        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("record 1 has 12, expected 11"));
    }

    #[test]
    fn test_invalid_magic() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.idx");
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"XXXX");
        std::fs::write(&path, header).unwrap();

        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Invalid .idx magic"));
    }

    #[test]
    fn test_unsupported_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.idx");
        let mut header = [0u8; 16];
        header[0..4].copy_from_slice(b"PFXI");
        header[4..6].copy_from_slice(&99u16.to_le_bytes());
        std::fs::write(&path, header).unwrap();

        let err = IdxReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Unsupported .idx version"));
    }
}
