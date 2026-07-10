use std::path::PathBuf;

use crate::benchmark::cli::next_value;
use crate::errors::ToolError;

use super::LineMatrixArchiveOptions;

pub fn parse_export_line_matrix_archive_args(
    args: Vec<String>,
) -> Result<LineMatrixArchiveOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut gto_data_version = None;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--gto-data-version" => {
                gto_data_version = Some(next_value(&args, &mut index)?.to_owned())
            }
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown export-line-matrix-archive option: {option}"
                )))
            }
        }
        index += 1;
    }

    Ok(LineMatrixArchiveOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        gto_data_version: gto_data_version
            .ok_or_else(|| ToolError::invalid_argument("--gto-data-version is required"))?,
        overwrite,
    })
}
