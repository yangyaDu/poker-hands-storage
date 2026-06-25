use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuildManifest {
    pub format: String,
    pub version: u32,
    pub source_db_checksum: String,
    pub built_at: String,
    pub dimensions: Vec<ManifestDimension>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDimension {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub concrete_line_count: u32,
    pub pack_count: u32,
    pub status: Option<String>,
    pub error: Option<String>,
    pub bin_file: Option<String>,
    pub idx_file: Option<String>,
    pub bin_file_size_bytes: Option<u64>,
    pub idx_file_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryableDimension {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub idx_file: String,
    pub bin_file: String,
}

#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnsupportedFormat {
        format: String,
        version: u32,
    },
    MissingDimensionFile {
        strategy: String,
        player_count: u32,
        depth_bb: u32,
        file_kind: &'static str,
    },
}

pub fn load_manifest(path: &Path) -> Result<BuildManifest, ManifestError> {
    let raw = fs::read_to_string(path).map_err(ManifestError::Io)?;
    parse_manifest(&raw)
}

pub fn parse_manifest(raw: &str) -> Result<BuildManifest, ManifestError> {
    let manifest: BuildManifest = serde_json::from_str(raw).map_err(ManifestError::Json)?;
    validate_manifest_version(&manifest)?;
    Ok(manifest)
}

pub fn queryable_dimensions(
    manifest: &BuildManifest,
) -> Result<Vec<QueryableDimension>, ManifestError> {
    validate_manifest_version(manifest)?;

    let mut dimensions = Vec::new();
    for dimension in &manifest.dimensions {
        if dimension.status.as_deref() == Some("failed") {
            continue;
        }

        let idx_file =
            dimension
                .idx_file
                .clone()
                .ok_or_else(|| ManifestError::MissingDimensionFile {
                    strategy: dimension.strategy.clone(),
                    player_count: dimension.player_count,
                    depth_bb: dimension.depth_bb,
                    file_kind: "idxFile",
                })?;
        let bin_file =
            dimension
                .bin_file
                .clone()
                .ok_or_else(|| ManifestError::MissingDimensionFile {
                    strategy: dimension.strategy.clone(),
                    player_count: dimension.player_count,
                    depth_bb: dimension.depth_bb,
                    file_kind: "binFile",
                })?;

        dimensions.push(QueryableDimension {
            strategy: dimension.strategy.clone(),
            player_count: dimension.player_count,
            depth_bb: dimension.depth_bb,
            idx_file,
            bin_file,
        });
    }

    Ok(dimensions)
}

fn validate_manifest_version(manifest: &BuildManifest) -> Result<(), ManifestError> {
    if manifest.format == "PFSP" && manifest.version == 1 {
        Ok(())
    } else {
        Err(ManifestError::UnsupportedFormat {
            format: manifest.format.clone(),
            version: manifest.version,
        })
    }
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "Failed to read manifest: {error}"),
            Self::Json(error) => write!(f, "Failed to parse manifest JSON: {error}"),
            Self::UnsupportedFormat { format, version } => {
                write!(f, "Unsupported manifest format: {format} v{version}")
            }
            Self::MissingDimensionFile {
                strategy,
                player_count,
                depth_bb,
                file_kind,
            } => write!(
                f,
                "Manifest dimension {strategy}_{player_count}max_{depth_bb}BB is missing {file_kind}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
    {
      "format": "PFSP",
      "version": 1,
      "sourceDbChecksum": "abc",
      "builtAt": "2026-06-21T13:05:29.013Z",
      "dimensions": [
        {
          "strategy": "default",
          "playerCount": 6,
          "depthBb": 100,
          "concreteLineCount": 3737,
          "packCount": 3737,
          "status": "success",
          "error": null,
          "binFile": "ranges_default_6max_100BB.bin",
          "idxFile": "ranges_default_6max_100BB.idx",
          "binFileSizeBytes": 2172204,
          "idxFileSizeBytes": 82230
        },
        {
          "strategy": "default",
          "playerCount": 6,
          "depthBb": 200,
          "concreteLineCount": 0,
          "packCount": 0,
          "status": "failed",
          "error": "fixture failure"
        }
      ],
      "files": [
        "meta.db",
        "ranges_default_6max_100BB.idx",
        "ranges_default_6max_100BB.bin"
      ]
    }
    "#;

    #[test]
    fn parses_current_manifest_shape() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        assert_eq!(manifest.format, "PFSP");
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.dimensions.len(), 2);
        assert_eq!(
            manifest.dimensions[0].idx_file.as_deref(),
            Some("ranges_default_6max_100BB.idx")
        );
    }

    #[test]
    fn returns_only_queryable_dimensions() {
        let manifest = parse_manifest(MANIFEST).unwrap();
        let dimensions = queryable_dimensions(&manifest).unwrap();
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0].strategy, "default");
        assert_eq!(dimensions[0].player_count, 6);
        assert_eq!(dimensions[0].depth_bb, 100);
    }

    #[test]
    fn rejects_unsupported_manifest_version() {
        let raw = MANIFEST.replace("\"version\": 1", "\"version\": 2");
        let err = parse_manifest(&raw).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::UnsupportedFormat {
                format,
                version: 2
            } if format == "PFSP"
        ));
    }

    #[test]
    fn rejects_success_dimension_without_idx_file() {
        let raw = MANIFEST.replace("\"idxFile\": \"ranges_default_6max_100BB.idx\",", "");
        let manifest = parse_manifest(&raw).unwrap();
        let err = queryable_dimensions(&manifest).unwrap_err();
        assert!(matches!(
            err,
            ManifestError::MissingDimensionFile {
                file_kind: "idxFile",
                ..
            }
        ));
    }
}
