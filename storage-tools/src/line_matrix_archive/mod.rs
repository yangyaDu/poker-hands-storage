use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use prost::Message;
use range_store_core::crc32c::{assert_crc32c, crc32c};
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::{Connection, Value};
use serde::{Deserialize, Serialize};

use crate::errors::ToolError;
use crate::line_matrix_export::convert::{build_line_matrix, validate_line_matrix};
use crate::line_matrix_export::proto::{HandEncoding, LineMatrix};
use crate::line_matrix_export::source::{load_all_lines, load_rows};

pub mod cli;
mod format;

use format::{
    read_header, read_index_record, write_header, write_index_record, IndexRecord, DATA_FILE_NAME,
    DATA_MAGIC, HEADER_SIZE, INDEX_FILE_NAME, INDEX_MAGIC, INDEX_RECORD_SIZE, MANIFEST_FILE_NAME,
    METADATA_FILE_NAME,
};

const ARCHIVE_FORMAT: &str = "LMSP";
const ARCHIVE_VERSION: u32 = 1;
const STRATEGY: &str = "default";
const PLAYER_COUNT: u32 = 6;
const DEPTH_BB: u32 = 100;

#[derive(Debug, Clone)]
pub struct LineMatrixArchiveOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub gto_data_version: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct LineMatrixArchiveSummary {
    pub matrix_count: u64,
    pub protobuf_bytes: u64,
    pub manifest_path: PathBuf,
    pub data_path: PathBuf,
    pub index_path: PathBuf,
    pub metadata_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LineMatrixArchive {
    data_path: PathBuf,
    index_path: PathBuf,
    matrix_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveManifest {
    format: String,
    version: u32,
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    gto_data_version: String,
    matrix_schema_version: u32,
    hand_encoding: String,
    matrix_count: u64,
    data_file: String,
    index_file: String,
    metadata_file: String,
    data_file_size_bytes: u64,
    index_file_size_bytes: u64,
    metadata_file_size_bytes: u64,
}

pub fn export_line_matrix_archive(
    options: &LineMatrixArchiveOptions,
) -> Result<LineMatrixArchiveSummary, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    if options.gto_data_version.trim().is_empty() {
        return Err(ToolError::invalid_argument(
            "--gto-data-version must not be empty",
        ));
    }

    let dimension = DimensionSpec::parse("default:6:100")?;
    let source = Connection::open(&options.source_db, true)?;
    let lines = load_all_lines(&source, &dimension)?;
    validate_dense_line_ids(&lines)?;
    prepare_output_dir(&options.out_dir, options.overwrite)?;

    let data_path = options.out_dir.join(DATA_FILE_NAME);
    let index_path = options.out_dir.join(INDEX_FILE_NAME);
    let metadata_path = options.out_dir.join(METADATA_FILE_NAME);
    let manifest_path = options.out_dir.join(MANIFEST_FILE_NAME);
    let data_tmp = data_path.with_extension("lmbin.tmp");
    let index_tmp = index_path.with_extension("lmidx.tmp");
    let metadata_tmp = metadata_path.with_extension("db.tmp");
    let manifest_tmp = manifest_path.with_extension("json.tmp");
    let temporary_paths = [&data_tmp, &index_tmp, &metadata_tmp, &manifest_tmp];
    for path in temporary_paths {
        remove_if_exists(path)?;
    }

    let build_result = build_archive_files(
        &source,
        &dimension,
        &lines,
        &options.gto_data_version,
        &data_tmp,
        &index_tmp,
        &metadata_tmp,
    );
    let (matrix_count, protobuf_bytes) = match build_result {
        Ok(summary) => summary,
        Err(error) => {
            for path in temporary_paths {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
    };

    let manifest = ArchiveManifest {
        format: ARCHIVE_FORMAT.to_owned(),
        version: ARCHIVE_VERSION,
        strategy: STRATEGY.to_owned(),
        player_count: PLAYER_COUNT,
        depth_bb: DEPTH_BB,
        gto_data_version: options.gto_data_version.clone(),
        matrix_schema_version: 1,
        hand_encoding: "HAND_ENCODING_169".to_owned(),
        matrix_count,
        data_file: DATA_FILE_NAME.to_owned(),
        index_file: INDEX_FILE_NAME.to_owned(),
        metadata_file: METADATA_FILE_NAME.to_owned(),
        data_file_size_bytes: fs::metadata(&data_tmp)?.len(),
        index_file_size_bytes: fs::metadata(&index_tmp)?.len(),
        metadata_file_size_bytes: fs::metadata(&metadata_tmp)?.len(),
    };
    write_manifest(&manifest_tmp, &manifest)?;

    for path in [&data_path, &index_path, &metadata_path, &manifest_path] {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    fs::rename(&data_tmp, &data_path)?;
    fs::rename(&index_tmp, &index_path)?;
    fs::rename(&metadata_tmp, &metadata_path)?;
    fs::rename(&manifest_tmp, &manifest_path)?;

    Ok(LineMatrixArchiveSummary {
        matrix_count,
        protobuf_bytes,
        manifest_path,
        data_path,
        index_path,
        metadata_path,
    })
}

impl LineMatrixArchive {
    pub fn open(dir: &Path) -> Result<Self, ToolError> {
        let manifest_path = dir.join(MANIFEST_FILE_NAME);
        let manifest: ArchiveManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .map_err(|error| ToolError::invalid_format(error.to_string()))?;
        validate_manifest(&manifest)?;

        let data_path = dir.join(&manifest.data_file);
        let index_path = dir.join(&manifest.index_file);
        let metadata_path = dir.join(&manifest.metadata_file);
        if !metadata_path.is_file() {
            return Err(ToolError::invalid_format(format!(
                "Archive metadata file does not exist: {}",
                metadata_path.display()
            )));
        }
        let mut data = File::open(&data_path)?;
        let mut index = File::open(&index_path)?;
        let data_count = read_header(&mut data, DATA_MAGIC)?;
        let index_count = read_header(&mut index, INDEX_MAGIC)?;
        if data_count != manifest.matrix_count || index_count != manifest.matrix_count {
            return Err(ToolError::invalid_format(
                "Archive record counts differ between manifest and binary files",
            ));
        }
        let expected_index_size = (HEADER_SIZE as u64)
            .checked_add(
                manifest
                    .matrix_count
                    .checked_mul(INDEX_RECORD_SIZE as u64)
                    .ok_or_else(|| ToolError::invalid_format("Archive index size overflow"))?,
            )
            .ok_or_else(|| ToolError::invalid_format("Archive index size overflow"))?;
        if fs::metadata(&index_path)?.len() != expected_index_size {
            return Err(ToolError::invalid_format(
                "Archive index file size is invalid",
            ));
        }

        Ok(Self {
            data_path,
            index_path,
            matrix_count: manifest.matrix_count,
        })
    }

    pub fn matrix_count(&self) -> u64 {
        self.matrix_count
    }

    pub fn read_matrix(&self, concrete_line_id: u64) -> Result<LineMatrix, ToolError> {
        if concrete_line_id == 0 || concrete_line_id > self.matrix_count {
            return Err(ToolError::new(
                "LINE_NOT_FOUND",
                format!("Concrete line {concrete_line_id} is not in this archive"),
            ));
        }
        let mut index = File::open(&self.index_path)?;
        let record = read_index_record(&mut index, concrete_line_id)?;
        let data_len = fs::metadata(&self.data_path)?.len();
        let payload_end = record
            .offset
            .checked_add(u64::from(record.byte_length))
            .ok_or_else(|| ToolError::invalid_format("Archive payload offset overflow"))?;
        if record.offset < HEADER_SIZE as u64 || payload_end > data_len {
            return Err(ToolError::invalid_format(
                "Archive index record points outside data file",
            ));
        }

        let mut payload = vec![0u8; record.byte_length as usize];
        let mut data = File::open(&self.data_path)?;
        data.seek(SeekFrom::Start(record.offset))?;
        data.read_exact(&mut payload)?;
        assert_crc32c(&payload, record.crc32c).map_err(ToolError::invalid_format)?;
        let matrix = LineMatrix::decode(payload.as_slice())
            .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
        validate_line_matrix(&matrix)?;
        Ok(matrix)
    }
}

fn build_archive_files(
    source: &Connection,
    dimension: &DimensionSpec,
    lines: &[crate::line_matrix_export::source::ResolvedLine],
    gto_data_version: &str,
    data_tmp: &Path,
    index_tmp: &Path,
    metadata_tmp: &Path,
) -> Result<(u64, u64), ToolError> {
    let mut data = create_new_file(data_tmp)?;
    let mut index = create_new_file(index_tmp)?;
    write_header(&mut data, DATA_MAGIC, 0)?;
    write_header(&mut index, INDEX_MAGIC, 0)?;

    let metadata = Connection::open(metadata_tmp, false)?;
    init_metadata_db(&metadata)?;
    metadata.exec("BEGIN")?;
    let result = (|| {
        let mut offset = HEADER_SIZE as u64;
        let mut protobuf_bytes = 0u64;
        for line in lines {
            let rows = load_rows(source, dimension, line.concrete_line_id)?;
            let (matrix, _) = build_line_matrix(&rows, gto_data_version)?;
            let payload = matrix.encode_to_vec();
            let decoded = LineMatrix::decode(payload.as_slice())
                .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
            validate_line_matrix(&decoded)?;
            if decoded != matrix {
                return Err(ToolError::new(
                    "PROTOBUF_ROUNDTRIP_MISMATCH",
                    "Decoded LineMatrix differs from the encoded matrix",
                ));
            }
            let byte_length = u32::try_from(payload.len()).map_err(|_| {
                ToolError::invalid_format("LineMatrix payload exceeds the archive u32 length limit")
            })?;
            data.write_all(&payload)?;
            write_index_record(
                &mut index,
                IndexRecord {
                    offset,
                    byte_length,
                    crc32c: crc32c(&payload),
                },
            )?;
            metadata.execute(
                "INSERT INTO concrete_lines(concrete_line_id, abstract_line, concrete_line)
                 VALUES (?1, ?2, ?3)",
                &[
                    Value::from(line.concrete_line_id),
                    Value::from(line.abstract_line.as_str()),
                    Value::from(line.concrete_line.as_str()),
                ],
            )?;
            offset = offset
                .checked_add(u64::from(byte_length))
                .ok_or_else(|| ToolError::invalid_format("Archive data offset overflow"))?;
            protobuf_bytes = protobuf_bytes
                .checked_add(u64::from(byte_length))
                .ok_or_else(|| ToolError::invalid_format("Archive payload size overflow"))?;
        }
        let matrix_count = u64::try_from(lines.len())
            .map_err(|_| ToolError::invalid_format("Archive matrix count exceeds u64"))?;
        metadata.exec("COMMIT")?;
        write_header(&mut data, DATA_MAGIC, matrix_count)?;
        write_header(&mut index, INDEX_MAGIC, matrix_count)?;
        data.sync_all()?;
        index.sync_all()?;
        Ok((matrix_count, protobuf_bytes))
    })();
    if result.is_err() {
        let _ = metadata.exec("ROLLBACK");
    }
    drop(metadata);
    result
}

fn validate_dense_line_ids(
    lines: &[crate::line_matrix_export::source::ResolvedLine],
) -> Result<(), ToolError> {
    for (index, line) in lines.iter().enumerate() {
        let expected = u32::try_from(index + 1)
            .map_err(|_| ToolError::invalid_format("Too many concrete lines for u32 ids"))?;
        if line.concrete_line_id != expected {
            return Err(ToolError::new(
                "NON_DENSE_CONCRETE_LINE_IDS",
                format!(
                    "Expected concrete_line_id={expected}, got {}",
                    line.concrete_line_id
                ),
            ));
        }
    }
    Ok(())
}

fn prepare_output_dir(out_dir: &Path, overwrite: bool) -> Result<(), ToolError> {
    fs::create_dir_all(out_dir)?;
    let artifacts = [
        out_dir.join(DATA_FILE_NAME),
        out_dir.join(INDEX_FILE_NAME),
        out_dir.join(METADATA_FILE_NAME),
        out_dir.join(MANIFEST_FILE_NAME),
    ];
    if !overwrite {
        if let Some(existing) = artifacts.iter().find(|path| path.exists()) {
            return Err(ToolError::invalid_argument(format!(
                "Archive output already exists: {}. Use --overwrite to replace it",
                existing.display()
            )));
        }
    }
    Ok(())
}

fn init_metadata_db(connection: &Connection) -> Result<(), ToolError> {
    connection.exec(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE concrete_lines (
           concrete_line_id INTEGER PRIMARY KEY,
           abstract_line TEXT NOT NULL,
           concrete_line TEXT NOT NULL,
           UNIQUE(abstract_line, concrete_line)
         );
         CREATE INDEX idx_concrete_lines_concrete_line ON concrete_lines(concrete_line);",
    )?;
    Ok(())
}

fn write_manifest(path: &Path, manifest: &ArchiveManifest) -> Result<(), ToolError> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

fn validate_manifest(manifest: &ArchiveManifest) -> Result<(), ToolError> {
    if manifest.format != ARCHIVE_FORMAT || manifest.version != ARCHIVE_VERSION {
        return Err(ToolError::invalid_format(
            "Unsupported LineMatrix archive manifest",
        ));
    }
    if manifest.strategy != STRATEGY
        || manifest.player_count != PLAYER_COUNT
        || manifest.depth_bb != DEPTH_BB
    {
        return Err(ToolError::invalid_format(
            "This archive reader only supports default:6:100",
        ));
    }
    if manifest.matrix_schema_version != 1
        || manifest.hand_encoding != HandEncoding::HandEncoding169.as_str_name()
    {
        return Err(ToolError::invalid_format(
            "Unsupported LineMatrix payload schema",
        ));
    }
    if manifest.gto_data_version.trim().is_empty() {
        return Err(ToolError::invalid_format(
            "Archive gtoDataVersion must not be empty",
        ));
    }
    if manifest.data_file != DATA_FILE_NAME
        || manifest.index_file != INDEX_FILE_NAME
        || manifest.metadata_file != METADATA_FILE_NAME
    {
        return Err(ToolError::invalid_format("Archive file names are invalid"));
    }
    Ok(())
}

fn create_new_file(path: &Path) -> Result<File, ToolError> {
    OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(path)
        .map_err(ToolError::from)
}

fn remove_if_exists(path: &Path) -> Result<(), ToolError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
