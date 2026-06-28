use std::path::PathBuf;

use crate::errors::AppError;

use super::report::VerifyMode;

#[derive(Debug, Clone)]
pub struct VerifyCommand {
    pub mode: VerifyMode,
    pub dir: PathBuf,
    pub source: Option<PathBuf>,
    pub verify_checksums: bool,
    pub sample_size: usize,
    pub max_failures: usize,
    pub out_path: PathBuf,
    pub md_path: PathBuf,
}

pub fn parse_verify_args(args: Vec<String>) -> Result<VerifyCommand, AppError> {
    let mut mode = VerifyMode::Standalone;
    let mut dir = None;
    let mut source = None;
    let mut verify_checksums = false;
    let mut sample_size = None;
    let mut max_failures = 50usize;
    let mut out_path = None;
    let mut md_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mode" => {
                mode = parse_verify_mode(next_value(&args, &mut index)?)?;
            }
            "--dir" => dir = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--source" => source = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--verify-checksum" => verify_checksums = true,
            "--sample-size" => {
                sample_size = Some(parse_usize(
                    "--sample-size",
                    next_value(&args, &mut index)?,
                )?)
            }
            "--max-failures" => {
                max_failures = parse_usize("--max-failures", next_value(&args, &mut index)?)?;
                if max_failures == 0 {
                    return Err(AppError::invalid_argument(
                        "--max-failures must be a positive integer",
                    ));
                }
            }
            "--out" => out_path = Some(PathBuf::from(next_value(&args, &mut index)?)),
            "--md" => md_path = Some(PathBuf::from(next_value(&args, &mut index)?)),
            option => {
                return Err(AppError::invalid_argument(format!(
                    "Unknown verify option: {option}"
                )))
            }
        }
        index += 1;
    }
    let dir = dir.ok_or_else(|| AppError::invalid_argument("--dir is required"))?;
    if mode == VerifyMode::Cross && source.is_none() {
        return Err(AppError::invalid_argument(
            "--source is required for cross mode",
        ));
    }
    let sample_size = sample_size.unwrap_or(if mode == VerifyMode::Cross { 10_000 } else { 0 });
    let out_path = out_path.unwrap_or_else(|| match mode {
        VerifyMode::Standalone => PathBuf::from("reports/range-strata-verify-standalone.json"),
        VerifyMode::Cross => PathBuf::from("reports/range-strata-verify-cross.json"),
    });
    let md_path = md_path.unwrap_or_else(|| match mode {
        VerifyMode::Standalone => PathBuf::from("reports/range-strata-verify-standalone.md"),
        VerifyMode::Cross => PathBuf::from("reports/range-strata-verify-cross.md"),
    });
    Ok(VerifyCommand {
        mode,
        dir,
        source,
        verify_checksums,
        sample_size,
        max_failures,
        out_path,
        md_path,
    })
}

fn parse_verify_mode(value: &str) -> Result<VerifyMode, AppError> {
    match value {
        "standalone" => Ok(VerifyMode::Standalone),
        "cross" => Ok(VerifyMode::Cross),
        _ => Err(AppError::invalid_argument(format!(
            "Invalid --mode value: {value}. Use standalone or cross."
        ))),
    }
}

fn next_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str, AppError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .ok_or_else(|| AppError::invalid_argument("Missing option value"))
}

fn parse_usize(name: &str, value: &str) -> Result<usize, AppError> {
    value
        .parse()
        .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
}
