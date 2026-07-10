use std::path::PathBuf;

use crate::benchmark::cli::next_value;
use crate::errors::ToolError;

use super::CompactLineMatrixArchiveOptions;

pub fn parse_export_compact_line_matrix_archive_args(
    args: Vec<String>,
) -> Result<CompactLineMatrixArchiveOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown export-compact-line-matrix-archive option: {option}"
                )))
            }
        }
        index += 1;
    }
    Ok(CompactLineMatrixArchiveOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        overwrite,
    })
}
