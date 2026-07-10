use std::path::PathBuf;

use range_store_core::dimension::DimensionSpec;

use crate::benchmark::cli::{next_value, parse_u32};
use crate::errors::ToolError;

use super::{ConcreteLineSelector, ExportLineMatrixOptions};

pub fn parse_export_line_matrix_args(
    args: Vec<String>,
) -> Result<ExportLineMatrixOptions, ToolError> {
    let mut source_db = None;
    let mut out_dir = None;
    let mut dimension = None;
    let mut concrete_line_id = None;
    let mut concrete_line = None;
    let mut abstract_line = None;
    let mut gto_data_version = None;
    let mut overwrite = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--source-db" => source_db = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--out-dir" => out_dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--dimension" => {
                dimension = Some(DimensionSpec::parse(next_value(&args, &mut index)?)?)
            }
            "--concrete-line-id" => {
                concrete_line_id = Some(parse_u32(
                    "--concrete-line-id",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--concrete-line" => concrete_line = Some(next_value(&args, &mut index)?.to_owned()),
            "--abstract-line" => abstract_line = Some(next_value(&args, &mut index)?.to_owned()),
            "--gto-data-version" => {
                gto_data_version = Some(next_value(&args, &mut index)?.to_owned())
            }
            "--overwrite" => overwrite = true,
            option => {
                return Err(ToolError::invalid_argument(format!(
                    "Unknown export-line-matrix option: {option}"
                )))
            }
        }
        index += 1;
    }

    let selector = match (concrete_line_id, concrete_line) {
        (Some(id), None) => {
            if abstract_line.is_some() {
                return Err(ToolError::invalid_argument(
                    "--abstract-line can only be used with --concrete-line",
                ));
            }
            ConcreteLineSelector::Id(id)
        }
        (None, Some(concrete_line)) => ConcreteLineSelector::Text {
            concrete_line,
            abstract_line,
        },
        (Some(_), Some(_)) => {
            return Err(ToolError::invalid_argument(
                "Use exactly one of --concrete-line-id and --concrete-line",
            ))
        }
        (None, None) => {
            return Err(ToolError::invalid_argument(
                "One of --concrete-line-id or --concrete-line is required",
            ))
        }
    };

    Ok(ExportLineMatrixOptions {
        source_db: source_db
            .ok_or_else(|| ToolError::invalid_argument("--source-db is required"))?,
        out_dir: out_dir.ok_or_else(|| ToolError::invalid_argument("--out-dir is required"))?,
        dimension: dimension
            .ok_or_else(|| ToolError::invalid_argument("--dimension is required"))?,
        selector,
        gto_data_version: gto_data_version
            .ok_or_else(|| ToolError::invalid_argument("--gto-data-version is required"))?,
        overwrite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_selector_with_abstract_line() {
        let options = parse_export_line_matrix_args(vec![
            "--source-db".to_owned(),
            "range.db".to_owned(),
            "--out-dir".to_owned(),
            "out".to_owned(),
            "--dimension".to_owned(),
            "default:6:100".to_owned(),
            "--concrete-line".to_owned(),
            "F-F-F".to_owned(),
            "--abstract-line".to_owned(),
            "F-F-F".to_owned(),
            "--gto-data-version".to_owned(),
            "poc-001".to_owned(),
        ])
        .expect("parse export args");

        assert_eq!(
            options.selector,
            ConcreteLineSelector::Text {
                concrete_line: "F-F-F".to_owned(),
                abstract_line: Some("F-F-F".to_owned()),
            }
        );
    }

    #[test]
    fn rejects_id_and_text_selectors_together() {
        let error = parse_export_line_matrix_args(vec![
            "--source-db".to_owned(),
            "range.db".to_owned(),
            "--out-dir".to_owned(),
            "out".to_owned(),
            "--dimension".to_owned(),
            "default:6:100".to_owned(),
            "--concrete-line-id".to_owned(),
            "1".to_owned(),
            "--concrete-line".to_owned(),
            "F-F-F".to_owned(),
            "--gto-data-version".to_owned(),
            "poc-001".to_owned(),
        ])
        .expect_err("selectors must be exclusive");

        assert_eq!(error.code(), "INVALID_ARGUMENT");
        assert!(error.message().contains("exactly one"));
    }
}
