use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use crate::errors::ToolError;

pub(crate) const DATA_FILE_NAME: &str = "matrices.lmbin";
pub(crate) const INDEX_FILE_NAME: &str = "matrices.lmidx";
pub(crate) const METADATA_FILE_NAME: &str = "lines.db";
pub(crate) const MANIFEST_FILE_NAME: &str = "manifest.json";
pub(crate) const DATA_MAGIC: &[u8; 4] = b"LMCN";
pub(crate) const INDEX_MAGIC: &[u8; 4] = b"LMCX";
pub(crate) const FORMAT_VERSION: u16 = 2;
pub(crate) const HEADER_SIZE: usize = 16;
pub(crate) const INDEX_RECORD_SIZE: usize = 16;

#[derive(Debug, Clone, Copy)]
pub(crate) struct IndexRecord {
    pub offset: u64,
    pub byte_length: u32,
    pub crc32c: u32,
}

pub(crate) fn write_header(
    file: &mut File,
    magic: &[u8; 4],
    record_count: u64,
) -> Result<(), ToolError> {
    let mut header = [0u8; HEADER_SIZE];
    header[0..4].copy_from_slice(magic);
    header[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&(HEADER_SIZE as u16).to_le_bytes());
    header[8..16].copy_from_slice(&record_count.to_le_bytes());
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header)?;
    Ok(())
}

pub(crate) fn read_header(file: &mut File, expected_magic: &[u8; 4]) -> Result<u64, ToolError> {
    let mut header = [0u8; HEADER_SIZE];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    if &header[0..4] != expected_magic {
        return Err(ToolError::invalid_format(
            "Compact LineMatrix archive magic does not match",
        ));
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != FORMAT_VERSION {
        return Err(ToolError::invalid_format(format!(
            "Unsupported Compact LineMatrix archive format version {version}"
        )));
    }
    let header_size = u16::from_le_bytes([header[6], header[7]]) as usize;
    if header_size != HEADER_SIZE {
        return Err(ToolError::invalid_format(format!(
            "Invalid Compact LineMatrix archive header size {header_size}"
        )));
    }
    Ok(u64::from_le_bytes(
        header[8..16].try_into().expect("header count"),
    ))
}

pub(crate) fn write_index_record(file: &mut File, record: IndexRecord) -> Result<(), ToolError> {
    let mut encoded = [0u8; INDEX_RECORD_SIZE];
    encoded[0..8].copy_from_slice(&record.offset.to_le_bytes());
    encoded[8..12].copy_from_slice(&record.byte_length.to_le_bytes());
    encoded[12..16].copy_from_slice(&record.crc32c.to_le_bytes());
    file.write_all(&encoded)?;
    Ok(())
}

pub(crate) fn read_index_record_from_slice(
    bytes: &[u8],
    concrete_line_id: u64,
) -> Result<IndexRecord, ToolError> {
    if concrete_line_id == 0 {
        return Err(ToolError::invalid_argument(
            "concrete_line_id must be at least 1",
        ));
    }
    let position = (HEADER_SIZE as u64)
        .checked_add(
            (concrete_line_id - 1)
                .checked_mul(INDEX_RECORD_SIZE as u64)
                .ok_or_else(|| {
                    ToolError::invalid_format("Compact archive index offset overflow")
                })?,
        )
        .ok_or_else(|| ToolError::invalid_format("Compact archive index offset overflow"))?;
    let start = usize::try_from(position)
        .map_err(|_| ToolError::invalid_format("Compact archive index offset exceeds usize"))?;
    let end = start
        .checked_add(INDEX_RECORD_SIZE)
        .ok_or_else(|| ToolError::invalid_format("Compact archive index offset overflow"))?;
    let encoded = bytes
        .get(start..end)
        .ok_or_else(|| ToolError::invalid_format("Compact archive index record is truncated"))?;
    Ok(IndexRecord {
        offset: u64::from_le_bytes(encoded[0..8].try_into().expect("record offset")),
        byte_length: u32::from_le_bytes(encoded[8..12].try_into().expect("record length")),
        crc32c: u32::from_le_bytes(encoded[12..16].try_into().expect("record checksum")),
    })
}
