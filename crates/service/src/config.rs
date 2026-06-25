use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::AppError;
use crate::naming::DimensionRef;

const DEFAULT_BIND: &str = "0.0.0.0:8080";
const DEFAULT_DATA_DIR: &str = "/data";
const DEFAULT_MAX_OPEN_HANDLES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceConfig {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub meta_db: PathBuf,
    pub max_open_handles: usize,
    pub verify_checksums: bool,
    pub prewarm: Vec<DimensionRef>,
}

impl ServiceConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|name| env::var(name).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Result<Self, AppError> {
        let bind_value = lookup("PHS_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let bind = bind_value.parse().map_err(|_| {
            AppError::invalid_argument(format!(
                "PHS_BIND is not a valid socket address: {bind_value}"
            ))
        })?;

        let data_dir =
            PathBuf::from(lookup("PHS_DATA_DIR").unwrap_or_else(|| DEFAULT_DATA_DIR.to_owned()));
        let meta_db = lookup("PHS_META_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("meta.db"));

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
        let prewarm = parse_prewarm(lookup("PHS_PREWARM").as_deref().unwrap_or_default())?;

        Ok(Self {
            bind,
            data_dir,
            meta_db,
            max_open_handles,
            verify_checksums,
            prewarm,
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config_from(values: &[(&str, &str)]) -> Result<ServiceConfig, AppError> {
        let values: HashMap<_, _> = values
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        ServiceConfig::from_lookup(|name| values.get(name).cloned())
    }

    #[test]
    fn uses_documented_defaults() {
        let config = config_from(&[]).unwrap();
        assert_eq!(config.bind, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.data_dir, PathBuf::from("/data"));
        assert_eq!(config.meta_db, PathBuf::from("/data/meta.db"));
        assert_eq!(config.max_open_handles, 3);
        assert!(!config.verify_checksums);
        assert!(config.prewarm.is_empty());
    }

    #[test]
    fn parses_overrides_and_prewarm_dimensions() {
        let config = config_from(&[
            ("PHS_BIND", "127.0.0.1:9090"),
            ("PHS_DATA_DIR", "data/store"),
            ("PHS_META_DB", "data/meta/custom.db"),
            ("PHS_MAX_OPEN_HANDLES", "5"),
            ("PHS_VERIFY_CHECKSUMS", "true"),
            ("PHS_PREWARM", "default:6:100,default:9:200"),
        ])
        .unwrap();
        assert_eq!(config.bind, "127.0.0.1:9090".parse().unwrap());
        assert_eq!(config.meta_db, PathBuf::from("data/meta/custom.db"));
        assert_eq!(config.max_open_handles, 5);
        assert!(config.verify_checksums);
        assert_eq!(
            config.prewarm,
            vec![
                DimensionRef::new("default", 6, 100),
                DimensionRef::new("default", 9, 200),
            ]
        );
    }

    #[test]
    fn rejects_invalid_values() {
        assert!(config_from(&[("PHS_MAX_OPEN_HANDLES", "0")]).is_err());
        assert!(config_from(&[("PHS_VERIFY_CHECKSUMS", "sometimes")]).is_err());
        assert!(config_from(&[("PHS_PREWARM", "default:6")]).is_err());
    }
}
