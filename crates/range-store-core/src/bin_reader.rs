//! .bin file reader — mmap the entire binary pack file.
//!
//! The .bin file starts with a 16-byte PFSP header, followed by concatenated
//! range packs. Each pack's offset and length come from the .idx file.

use std::fs::File;
use std::io;
use std::path::Path;

use memmap2::Mmap;

use crate::types::{PfspHeader, PFSP_HEADER_SIZE, PFSP_MAGIC};

/// Owned mmap of a .bin file with validated PFSP header.
#[derive(Debug)]
pub struct BinReader {
    _file: File,
    mmap: Mmap,
}

impl BinReader {
    /// Open and mmap the .bin file at `path`, validating the PFSP header.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;

        let meta = file.metadata()?;
        let file_len = meta.len();
        if file_len < PFSP_HEADER_SIZE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(".bin file too small: {} bytes", file_len),
            ));
        }

        // SAFETY: the file is opened read-only, has already been checked to be
        // at least large enough for the fixed PFSP header, and the `File` is
        // kept alive in `BinReader` for the full lifetime of the mmap. All pack
        // reads validate offset/length against `mmap.len()` before slicing.
        //
        // This assumes generated store files are immutable while a
        // `DimensionHandle` is alive. An external process that truncates or
        // mutates the same file can still invalidate the OS mapping (for
        // example SIGBUS on Unix); callers should deploy by writing new
        // versioned directories and swapping handles instead of modifying files
        // in place.
        let mmap = unsafe { Mmap::map(&file)? };

        // Validate header
        Self::validate_header(&mmap)?;

        Ok(Self { _file: file, mmap })
    }

    /// Return a view of the pack data at `offset..offset + byte_length`.
    ///
    /// `offset` must be >= PFSP_HEADER_SIZE (the data starts after the header).
    #[inline]
    pub fn read_pack(&self, offset: u32, byte_length: u32) -> io::Result<&[u8]> {
        let start = offset as usize;
        let len = byte_length as usize;
        let end = start.checked_add(len).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid pack range: offset {} + byte length {} overflows",
                    offset, byte_length
                ),
            )
        })?;

        if start < PFSP_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Invalid pack offset: {}, expected >= {}",
                    offset, PFSP_HEADER_SIZE
                ),
            ));
        }

        if end > self.mmap.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Pack range out of bounds: offset {}, byte length {}, file length {}",
                    offset,
                    byte_length,
                    self.mmap.len()
                ),
            ));
        }

        Ok(&self.mmap[start..end])
    }
}

impl BinReader {
    fn validate_header(mmap: &[u8]) -> io::Result<PfspHeader> {
        if mmap[0..4] != PFSP_MAGIC[..] {
            let magic_str = String::from_utf8_lossy(&mmap[0..4]);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid ranges.bin magic: {}, expected PFSP", magic_str),
            ));
        }

        let version = u16::from_le_bytes([mmap[4], mmap[5]]);
        if version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported PFSP version: {}", version),
            ));
        }

        let endian = mmap[6];
        if endian != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported endian, expected little-endian",
            ));
        }

        let float_type = mmap[7];
        if float_type != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported float type, expected float32",
            ));
        }

        let layout = mmap[8];
        if layout != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported layout, expected sparse hand-major v1",
            ));
        }

        let compression = mmap[9];
        if compression != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported compression, expected none",
            ));
        }

        let header_size = u16::from_le_bytes([mmap[10], mmap[11]]);
        if header_size as usize != PFSP_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unsupported header size: {}", header_size),
            ));
        }

        Ok(PfspHeader {
            version,
            endian,
            float_type,
            layout,
            compression,
            header_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_test_bin(path: &std::path::Path, extra_data: Option<&[u8]>) {
        let mut f = File::create(path).unwrap();
        let mut header = [0u8; PFSP_HEADER_SIZE];
        header[0..4].copy_from_slice(b"PFSP");
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[6] = 1;
        header[7] = 1;
        header[8] = 1;
        header[9] = 0;
        header[10..12].copy_from_slice(&(PFSP_HEADER_SIZE as u16).to_le_bytes());
        f.write_all(&header).unwrap();
        if let Some(extra) = extra_data {
            f.write_all(extra).unwrap();
        }
        f.flush().unwrap();
    }

    #[test]
    fn test_valid_header() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.bin");
        write_test_bin(&path, None);
        let reader = BinReader::open(&path).unwrap();
        drop(reader);
    }

    #[test]
    fn test_invalid_magic() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.bin");
        let mut header = [0u8; PFSP_HEADER_SIZE];
        header[0..4].copy_from_slice(b"XXXX");
        std::fs::write(&path, header).unwrap();

        let err = BinReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Invalid ranges.bin magic"));
    }

    #[test]
    fn test_invalid_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.bin");
        let mut header = [0u8; PFSP_HEADER_SIZE];
        header[0..4].copy_from_slice(b"PFSP");
        header[4..6].copy_from_slice(&99u16.to_le_bytes());
        header[6] = 1;
        header[7] = 1;
        header[8] = 1;
        header[9] = 0;
        header[10..12].copy_from_slice(&(PFSP_HEADER_SIZE as u16).to_le_bytes());
        std::fs::write(&path, header).unwrap();

        let err = BinReader::open(&path).unwrap_err();
        assert!(err.to_string().contains("Unsupported PFSP version"));
    }

    #[test]
    fn test_read_pack() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.bin");
        let extra = vec![0x42u8; 100];
        write_test_bin(&path, Some(&extra));

        let reader = BinReader::open(&path).unwrap();
        let pack = reader.read_pack(PFSP_HEADER_SIZE as u32, 100).unwrap();
        assert_eq!(pack.len(), 100);
        assert_eq!(pack[0], 0x42);
        drop(reader);
    }

    #[test]
    fn test_read_pack_rejects_header_offset() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.bin");
        let extra = vec![0x42u8; 100];
        write_test_bin(&path, Some(&extra));

        let reader = BinReader::open(&path).unwrap();
        let err = reader
            .read_pack((PFSP_HEADER_SIZE - 1) as u32, 1)
            .unwrap_err();
        assert!(err.to_string().contains("Invalid pack offset"));
    }

    #[test]
    fn test_read_pack_rejects_out_of_bounds_range() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.bin");
        let extra = vec![0x42u8; 100];
        write_test_bin(&path, Some(&extra));

        let reader = BinReader::open(&path).unwrap();
        let err = reader
            .read_pack(PFSP_HEADER_SIZE as u32 + 50, 100)
            .unwrap_err();
        assert!(err.to_string().contains("Pack range out of bounds"));
    }
}
