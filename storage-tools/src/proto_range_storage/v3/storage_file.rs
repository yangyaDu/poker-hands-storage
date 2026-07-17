use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use range_store_core::crc32c::crc32c;

use crate::errors::ToolError;

use super::format::{
    encode_header, encode_payload_locator, encode_section_descriptor, FileHeader, FileKind,
    PayloadLocator, SectionDescriptor, SectionKind, HEADER_SIZE, PAYLOAD_LOCATOR_SIZE,
    SECTION_DESCRIPTOR_SIZE,
};
use super::manifest::ManifestFile;

/// The two files that make up one V3 dataset while it is being exported.
///
/// The final paths stay hidden behind this module until both temporary files have been written.
/// This keeps publication mechanics out of individual dataset exporters.
pub(super) struct StagedFilePair {
    pub(super) data: StagedFile,
    pub(super) index: StagedFile,
}

impl StagedFilePair {
    pub(super) fn new(dir: &Path, data_file_name: &str, index_file_name: &str) -> Self {
        Self {
            data: StagedFile::new(dir, data_file_name),
            index: StagedFile::new(dir, index_file_name),
        }
    }

    pub(super) fn final_paths(&self) -> [&Path; 2] {
        [&self.data.final_path, &self.index.final_path]
    }

    pub(super) fn remove_temporary_files(&self) {
        self.data.remove_temporary_file();
        self.index.remove_temporary_file();
    }

    pub(super) fn publish(&self, overwrite: bool) -> Result<(), ToolError> {
        if overwrite {
            self.remove_final_files()?;
        }
        self.publish_temporary_files()
    }

    pub(super) fn remove_final_files(&self) -> Result<(), ToolError> {
        self.data.remove_final_file()?;
        self.index.remove_final_file()?;
        Ok(())
    }

    pub(super) fn publish_temporary_files(&self) -> Result<(), ToolError> {
        fs::rename(&self.data.temporary_path, &self.data.final_path)?;
        fs::rename(&self.index.temporary_path, &self.index.final_path)?;
        Ok(())
    }
}

pub(super) struct StagedFile {
    pub(super) final_path: PathBuf,
    pub(super) temporary_path: PathBuf,
}

impl StagedFile {
    fn new(dir: &Path, file_name: &str) -> Self {
        Self {
            final_path: dir.join(file_name),
            temporary_path: dir.join(format!("{file_name}.tmp")),
        }
    }

    fn remove_temporary_file(&self) {
        let _ = fs::remove_file(&self.temporary_path);
    }

    fn remove_final_file(&self) -> Result<(), ToolError> {
        if self.final_path.exists() {
            fs::remove_file(&self.final_path)?;
        }
        Ok(())
    }
}

pub(super) struct EncodedSection {
    pub(super) kind: SectionKind,
    pub(super) record_size: u16,
    pub(super) record_count: u64,
    pub(super) bytes: Vec<u8>,
}

pub(super) fn payload_locator_section(
    kind: SectionKind,
    locators: &[PayloadLocator],
) -> EncodedSection {
    let mut bytes = Vec::with_capacity(locators.len() * PAYLOAD_LOCATOR_SIZE);
    for locator in locators {
        bytes.extend_from_slice(&encode_payload_locator(*locator));
    }
    EncodedSection {
        kind,
        record_size: PAYLOAD_LOCATOR_SIZE as u16,
        record_count: locators.len() as u64,
        bytes,
    }
}

pub(super) fn write_payload_data_file(
    path: &Path,
    kind: FileKind,
    payload_count: usize,
    secondary_count: u64,
    mut encode_payload: impl FnMut(usize) -> Result<Vec<u8>, ToolError>,
    too_large_error: impl Fn() -> ToolError,
    offset_overflow_error: impl Fn() -> ToolError,
) -> Result<Vec<PayloadLocator>, ToolError> {
    let mut file = File::create(path)?;
    file.write_all(&encode_header(FileHeader::new(
        kind,
        payload_count as u64,
        secondary_count,
        0,
    )))?;
    let mut offset = HEADER_SIZE as u64;
    let mut locators = Vec::with_capacity(payload_count);
    for payload_index in 0..payload_count {
        let bytes = encode_payload(payload_index)?;
        let byte_length = u32::try_from(bytes.len()).map_err(|_| too_large_error())?;
        file.write_all(&bytes)?;
        locators.push(PayloadLocator {
            offset,
            byte_length,
            crc32c: crc32c(&bytes),
        });
        offset = offset
            .checked_add(u64::from(byte_length))
            .ok_or_else(&offset_overflow_error)?;
    }
    file.sync_all()?;
    Ok(locators)
}

pub(super) fn write_index_file(
    path: &Path,
    kind: FileKind,
    primary_count: u64,
    secondary_count: u64,
    sections: Vec<EncodedSection>,
) -> Result<(), ToolError> {
    let section_count = u32::try_from(sections.len())
        .map_err(|_| ToolError::invalid_format("V3 index section count exceeds uint32"))?;
    let directory_bytes = sections
        .len()
        .checked_mul(SECTION_DESCRIPTOR_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 section directory size overflow"))?;
    let mut offset = u64::try_from(HEADER_SIZE + directory_bytes)
        .map_err(|_| ToolError::invalid_format("V3 index section offset exceeds uint64"))?;
    let mut descriptors = Vec::with_capacity(sections.len());
    for section in &sections {
        if section.bytes.len()
            != usize::try_from(section.record_count)
                .ok()
                .and_then(|count| count.checked_mul(usize::from(section.record_size)))
                .ok_or_else(|| ToolError::invalid_format("V3 index section size overflow"))?
        {
            return Err(ToolError::invalid_format(
                "V3 encoded index section length does not match records",
            ));
        }
        let descriptor = SectionDescriptor::new(
            section.kind,
            section.record_size,
            offset,
            section.record_count,
        )?;
        offset = descriptor.end()?;
        descriptors.push(descriptor);
    }

    let mut file = File::create(path)?;
    file.write_all(&encode_header(FileHeader::new(
        kind,
        primary_count,
        secondary_count,
        section_count,
    )))?;
    for descriptor in descriptors {
        file.write_all(&encode_section_descriptor(descriptor))?;
    }
    for section in sections {
        file.write_all(&section.bytes)?;
    }
    file.sync_all()?;
    Ok(())
}

pub(super) fn manifest_file(
    path: &Path,
    file_name: &str,
    primary_count: u64,
    secondary_count: u64,
) -> Result<ManifestFile, ToolError> {
    let bytes = fs::read(path)?;
    Ok(ManifestFile {
        file_name: file_name.to_owned(),
        size_bytes: bytes.len() as u64,
        crc32c: crc32c(&bytes),
        primary_count,
        secondary_count,
    })
}
