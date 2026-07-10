use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use prost::Message;
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

pub(crate) mod convert;
mod report;
pub(crate) mod source;

pub mod cli;
pub mod proto;

use convert::{build_line_matrix, validate_line_matrix};
use proto::LineMatrix;
use source::{load_rows, resolve_line};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConcreteLineSelector {
    Id(u32),
    Text {
        concrete_line: String,
        abstract_line: Option<String>,
    },
}

impl fmt::Display for ConcreteLineSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(formatter, "concrete_line_id={id}"),
            Self::Text {
                concrete_line,
                abstract_line: Some(abstract_line),
            } => write!(
                formatter,
                "concrete_line={concrete_line:?}, abstract_line={abstract_line:?}"
            ),
            Self::Text {
                concrete_line,
                abstract_line: None,
            } => write!(formatter, "concrete_line={concrete_line:?}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExportLineMatrixOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub selector: ConcreteLineSelector,
    pub gto_data_version: String,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct ExportLineMatrixSummary {
    pub concrete_line_id: u32,
    pub abstract_line: String,
    pub concrete_line: String,
    pub action_count: usize,
    pub source_row_count: usize,
    pub null_ev_count: usize,
    pub hands_with_actions: usize,
    pub hands_without_actions: usize,
    pub frequency_sum_mismatch_hand_count: usize,
    pub max_frequency_error_x10000: u32,
    pub protobuf_bytes: usize,
    pub protobuf_path: PathBuf,
    pub debug_json_path: PathBuf,
    pub verify_json_path: PathBuf,
}

pub fn export_line_matrix(
    options: &ExportLineMatrixOptions,
) -> Result<ExportLineMatrixSummary, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }

    let connection = Connection::open(&options.source_db, true)?;
    let line = resolve_line(&connection, &options.dimension, &options.selector)?;
    let rows = load_rows(&connection, &options.dimension, line.concrete_line_id)?;
    let (matrix, stats) = build_line_matrix(&rows, &options.gto_data_version)?;

    let protobuf = matrix.encode_to_vec();
    let decoded = LineMatrix::decode(protobuf.as_slice())
        .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
    validate_line_matrix(&decoded)?;
    if decoded != matrix {
        return Err(ToolError::new(
            "PROTOBUF_ROUNDTRIP_MISMATCH",
            "Decoded LineMatrix differs from the encoded matrix",
        ));
    }

    let debug_json = report::debug_json(&line, &matrix)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    let verify_json = report::verify_json(&line, &matrix, &stats, protobuf.len())
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;

    fs::create_dir_all(&options.out_dir)?;
    let file_stem = format!("line-{}", line.concrete_line_id);
    let protobuf_path = options.out_dir.join(format!("{file_stem}.pb"));
    let debug_json_path = options.out_dir.join(format!("{file_stem}.debug.json"));
    let verify_json_path = options.out_dir.join(format!("{file_stem}.verify.json"));
    let artifacts = [
        (&protobuf_path, protobuf.as_slice()),
        (&debug_json_path, debug_json.as_slice()),
        (&verify_json_path, verify_json.as_slice()),
    ];
    if !options.overwrite {
        if let Some((path, _)) = artifacts.iter().find(|(path, _)| path.exists()) {
            return Err(ToolError::invalid_argument(format!(
                "Output already exists: {}. Use --overwrite to replace it",
                path.display()
            )));
        }
    }
    for (path, bytes) in artifacts {
        write_artifact(path, bytes, options.overwrite)?;
    }

    Ok(ExportLineMatrixSummary {
        concrete_line_id: line.concrete_line_id,
        abstract_line: line.abstract_line,
        concrete_line: line.concrete_line,
        action_count: matrix.actions.len(),
        source_row_count: stats.source_row_count,
        null_ev_count: stats.null_ev_count,
        hands_with_actions: stats.hands_with_actions,
        hands_without_actions: convert::HAND_COUNT_169 - stats.hands_with_actions,
        frequency_sum_mismatch_hand_count: stats.frequency_sum_mismatch_hand_count,
        max_frequency_error_x10000: stats.max_frequency_error_x10000,
        protobuf_bytes: protobuf.len(),
        protobuf_path,
        debug_json_path,
        verify_json_path,
    })
}

fn write_artifact(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), ToolError> {
    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
