use range_store_core::manifest::{parse_manifest, queryable_dimensions, ManifestError};

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
