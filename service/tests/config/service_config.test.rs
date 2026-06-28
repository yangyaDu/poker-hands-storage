use std::collections::HashMap;
use std::path::PathBuf;

use poker_hands_storage_service::config::ServiceConfig;
use poker_hands_storage_service::domain::dimension::DimensionRef;
use poker_hands_storage_service::errors::AppError;

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
