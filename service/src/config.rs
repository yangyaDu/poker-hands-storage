use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::errors::AppError;
use range_store_core::dimension::DimensionRef;

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_DATA_DIR: &str = "/data";
const DEFAULT_MAX_OPEN_HANDLES: usize = 2;
const DEFAULT_METADATA_CACHE_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_STRATEGY_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub max_open_handles: usize,
    /// Facade-wide metadata cache budget, dynamically divided among open dimension handles.
    pub metadata_cache_byte_budget: usize,
    /// Facade-wide decoded-strategy cache budget, dynamically divided among open dimension handles.
    pub strategy_cache_byte_budget: usize,
    pub verify_checksums: bool,
    pub prewarm: Vec<DimensionRef>,
}

impl ServiceConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self, AppError> {
        let bind_value = lookup("PHS_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let bind = bind_value.parse().map_err(|_| {
            AppError::invalid_argument(format!(
                "PHS_BIND is not a valid socket address: {bind_value}"
            ))
        })?;

        let data_dir =
            PathBuf::from(lookup("PHS_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned()));
        let max_open_handles = match lookup("PHS_MAX_OPEN_HANDLES") {
            Some(value) => {
                let parsed = value.parse::<usize>().map_err(|_| {
                    AppError::invalid_argument("PHS_MAX_OPEN_HANDLES must be a positive integer")
                })?;
                if parsed == 0 {
                    return Err(AppError::invalid_argument(
                        "PHS_MAX_OPEN_HANDLES must be a positive integer",
                    ));
                }
                parsed
            }
            None => DEFAULT_MAX_OPEN_HANDLES,
        };

        let verify_checksums = match lookup("PHS_VERIFY_CHECKSUMS") {
            Some(value) => parse_bool("PHS_VERIFY_CHECKSUMS", &value)?,
            None => false,
        };
        let metadata_cache_byte_budget = parse_usize_or_default(
            "PHS_METADATA_CACHE_BYTES",
            lookup("PHS_METADATA_CACHE_BYTES"),
            DEFAULT_METADATA_CACHE_BYTES,
        )?;
        let strategy_cache_byte_budget = parse_usize_or_default(
            "PHS_STRATEGY_CACHE_BYTES",
            lookup("PHS_STRATEGY_CACHE_BYTES"),
            DEFAULT_STRATEGY_CACHE_BYTES,
        )?;
        let prewarm = parse_prewarm(lookup("PHS_PREWARM").as_deref().unwrap_or_default())?;

        Ok(Self {
            bind,
            data_dir,
            max_open_handles,
            metadata_cache_byte_budget,
            strategy_cache_byte_budget,
            verify_checksums,
            prewarm,
        })
    }
}

fn parse_usize_or_default(
    name: &str,
    value: Option<String>,
    default: usize,
) -> Result<usize, AppError> {
    value
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| AppError::invalid_argument(format!("{name} must be an integer")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn parse_bool(name: &str, value: &str) -> Result<bool, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::invalid_argument(format!(
            "{name} must be true or false"
        ))),
    }
}

fn parse_prewarm(value: &str) -> Result<Vec<DimensionRef>, AppError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_dimension)
        .collect()
}

fn parse_dimension(value: &str) -> Result<DimensionRef, AppError> {
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 3 || parts[0].is_empty() {
        return Err(AppError::invalid_argument(format!(
            "Invalid prewarm dimension '{value}', expected strategy:player_count:depth_bb"
        )));
    }
    let player_count = parts[1].parse().map_err(|_| {
        AppError::invalid_argument(format!(
            "Invalid player count in prewarm dimension '{value}'"
        ))
    })?;
    let depth_bb = parts[2].parse().map_err(|_| {
        AppError::invalid_argument(format!("Invalid depth in prewarm dimension '{value}'"))
    })?;
    Ok(DimensionRef::new(parts[0], player_count, depth_bb))
}
