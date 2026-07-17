use std::fs;
use std::path::{Path, PathBuf};

use range_store_core::crc32c::crc32c;
use range_store_core::dimension::{discover_dimensions, DimensionSpec};
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

use super::manifest::{
    read_manifest, write_manifest, ArchiveManifest, ManifestFile, ARCHIVE_FORMAT, ARCHIVE_VERSION,
    PAYLOAD_SCHEMA, PREFLOP_HAND_ENCODING,
};
use super::metadata_store::{
    export_metadata, MetadataExportOptions, MetadataStore, MetadataStoreOptions,
    DEFAULT_METADATA_PAGE_TARGET_BYTES,
};
use super::strategy_store::{
    export_hand_strategies, HandStrategyExportOptions, HandStrategyStore, HandStrategyStoreOptions,
};
use super::verification::{cross_verify_sqlite_v3, verify_v3_archive, V3VerificationOptions};

#[derive(Debug, Clone)]
pub struct V3ArchiveExportOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub metadata_page_target_bytes: usize,
    pub overwrite: bool,
}

impl V3ArchiveExportOptions {
    pub fn new(source_db: PathBuf, out_dir: PathBuf, dimension: DimensionSpec) -> Self {
        Self {
            source_db,
            out_dir,
            dimension,
            metadata_page_target_bytes: DEFAULT_METADATA_PAGE_TARGET_BYTES,
            overwrite: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct V3ArchiveExportSummary {
    pub manifest: ArchiveManifest,
    pub archive_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct V3ArchivesExportOptions {
    pub source_db: PathBuf,
    pub out_root: PathBuf,
    pub metadata_page_target_bytes: usize,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct V3ArchivesExportSummary {
    pub archives: Vec<V3ArchiveExportSummary>,
}

#[derive(Debug, Clone, Copy)]
pub struct V3ArchiveOpenOptions {
    pub verify_file_checksums: bool,
    pub metadata_cache_byte_budget: usize,
    pub strategy_cache_byte_budget: usize,
}

impl Default for V3ArchiveOpenOptions {
    fn default() -> Self {
        Self {
            verify_file_checksums: false,
            metadata_cache_byte_budget: super::metadata_store::DEFAULT_METADATA_CACHE_BYTE_BUDGET,
            strategy_cache_byte_budget: super::strategy_store::DEFAULT_STRATEGY_CACHE_BYTE_BUDGET,
        }
    }
}

pub struct V3Archive {
    manifest: ArchiveManifest,
    metadata: MetadataStore,
    strategies: HandStrategyStore,
}

impl V3Archive {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(dir, V3ArchiveOpenOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: V3ArchiveOpenOptions,
    ) -> Result<Self, ToolError> {
        let dir = dir.as_ref();
        let manifest = read_manifest(dir)?;
        validate_manifest_files(dir, &manifest, options.verify_file_checksums)?;
        let metadata = MetadataStore::open_with_options(
            dir,
            MetadataStoreOptions {
                page_cache_byte_budget: options.metadata_cache_byte_budget,
            },
        )?;
        let strategies = HandStrategyStore::open_with_options(
            dir,
            HandStrategyStoreOptions {
                cache_byte_budget: options.strategy_cache_byte_budget,
            },
        )?;
        if strategies.record_count() != manifest.hand_strategies.record_count {
            return Err(ToolError::new(
                "INVALID_V3_MANIFEST",
                "Manifest hand strategy count does not match index",
            ));
        }
        Ok(Self {
            manifest,
            metadata,
            strategies,
        })
    }

    pub fn manifest(&self) -> &ArchiveManifest {
        &self.manifest
    }

    pub fn dimension(&self) -> DimensionSpec {
        DimensionSpec {
            strategy: self.manifest.strategy.clone(),
            player_count: self.manifest.player_count,
            depth_bb: self.manifest.depth_bb,
        }
    }

    pub fn metadata(&self) -> &MetadataStore {
        &self.metadata
    }

    pub fn strategies(&self) -> &HandStrategyStore {
        &self.strategies
    }

    pub(crate) fn resize_cache_budgets(
        &self,
        metadata_cache_byte_budget: usize,
        strategy_cache_byte_budget: usize,
    ) {
        self.metadata
            .resize_cache_budget(metadata_cache_byte_budget);
        self.strategies
            .resize_cache_budget(strategy_cache_byte_budget);
    }

    pub(crate) fn cache_budgets(&self) -> (usize, usize) {
        (
            self.metadata.cache_byte_budget(),
            self.strategies.cache_byte_budget(),
        )
    }
}

pub fn export_v3_archive(
    options: &V3ArchiveExportOptions,
) -> Result<V3ArchiveExportSummary, ToolError> {
    export_v3_archive_with_verifier(options, |building_dir| {
        verify_v3_archive_before_publish(building_dir)
    })
}

fn export_v3_archive_with_verifier<F>(
    options: &V3ArchiveExportOptions,
    verify_before_publish: F,
) -> Result<V3ArchiveExportSummary, ToolError>
where
    F: FnOnce(&Path) -> Result<(), ToolError>,
{
    if options.out_dir.exists() && !options.overwrite {
        return Err(ToolError::invalid_argument(format!(
            "V3 archive output already exists: {}",
            options.out_dir.display()
        )));
    }
    let building_dir = sibling_path(&options.out_dir, "building")?;
    remove_dir_if_exists(&building_dir)?;
    fs::create_dir_all(&building_dir)?;

    let result = (|| {
        let metadata = export_metadata(&MetadataExportOptions {
            source_db: options.source_db.clone(),
            out_dir: building_dir.clone(),
            dimension: options.dimension.clone(),
            page_target_bytes: options.metadata_page_target_bytes,
            overwrite: false,
        })?;
        let hand_strategies = export_hand_strategies(
            &HandStrategyExportOptions {
                source_db: options.source_db.clone(),
                out_dir: building_dir.clone(),
                dimension: options.dimension.clone(),
                overwrite: false,
            },
            &metadata.concrete_paths,
        )?;
        let manifest = ArchiveManifest {
            format: ARCHIVE_FORMAT.to_owned(),
            version: ARCHIVE_VERSION,
            payload_schema: PAYLOAD_SCHEMA.to_owned(),
            strategy: options.dimension.strategy.clone(),
            player_count: options.dimension.player_count,
            depth_bb: options.dimension.depth_bb,
            hand_encoding: PREFLOP_HAND_ENCODING.to_owned(),
            complete: true,
            drill_scenarios: metadata.drill_scenarios,
            abstract_action_paths: metadata.abstract_action_paths,
            hand_strategies,
        };
        write_manifest(&building_dir, &manifest)?;
        verify_before_publish(&building_dir)?;
        Ok::<ArchiveManifest, ToolError>(manifest)
    })();
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            let _ = fs::remove_dir_all(&building_dir);
            return Err(error);
        }
    };
    publish_directory(&building_dir, &options.out_dir, options.overwrite)?;
    Ok(V3ArchiveExportSummary {
        manifest,
        archive_dir: options.out_dir.clone(),
    })
}

/// Performs the complete standalone read-back verification required before an
/// archive manifest may be published to its final directory.
pub fn verify_v3_archive_before_publish(archive_dir: impl AsRef<Path>) -> Result<(), ToolError> {
    let archive_dir = archive_dir.as_ref();
    let report = verify_v3_archive(archive_dir, V3VerificationOptions::default());
    if report.ok {
        return Ok(());
    }
    Err(ToolError::verify(format!(
        "V3 archive standalone verification failed for {} with {} failures; first={:?}",
        archive_dir.display(),
        report.failure_count,
        report.failure_samples.first()
    )))
}

pub fn export_all_v3_archives(
    options: &V3ArchivesExportOptions,
) -> Result<V3ArchivesExportSummary, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    fs::create_dir_all(&options.out_root)?;
    let source = Connection::open(&options.source_db, true)?;
    let dimensions = discover_dimensions(&source)?;
    drop(source);
    if dimensions.is_empty() {
        return Err(ToolError::new(
            "V3_ARCHIVE_EMPTY",
            "Source database has no discoverable dimensions",
        ));
    }

    let mut archives = Vec::with_capacity(dimensions.len());
    for dimension in dimensions {
        let out_dir = options.out_root.join(format!(
            "{}_{}max_{}BB",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        ));
        let summary = export_v3_archive(&V3ArchiveExportOptions {
            source_db: options.source_db.clone(),
            out_dir,
            dimension,
            metadata_page_target_bytes: options.metadata_page_target_bytes,
            overwrite: options.overwrite,
        })?;
        let verification = cross_verify_sqlite_v3(
            &options.source_db,
            &summary.archive_dir,
            V3VerificationOptions::default(),
        );
        if !verification.ok {
            return Err(ToolError::verify(format!(
                "SQLite/V3 cross verification failed for {} with {} differences; first={:?}",
                summary.archive_dir.display(),
                verification.failure_count,
                verification.failure_samples.first()
            )));
        }
        archives.push(summary);
    }
    Ok(V3ArchivesExportSummary { archives })
}

fn validate_manifest_files(
    dir: &Path,
    manifest: &ArchiveManifest,
    verify_checksums: bool,
) -> Result<(), ToolError> {
    for file in manifest_files(manifest) {
        let path = dir.join(&file.file_name);
        let metadata = fs::metadata(&path).map_err(|error| {
            ToolError::new(
                "INVALID_V3_MANIFEST",
                format!("Missing manifest file {}: {error}", path.display()),
            )
        })?;
        if metadata.len() != file.size_bytes {
            return Err(ToolError::new(
                "INVALID_V3_MANIFEST",
                format!("Manifest size mismatch for {}", file.file_name),
            ));
        }
        if verify_checksums && crc32c(&fs::read(&path)?) != file.crc32c {
            return Err(ToolError::new(
                "INVALID_V3_MANIFEST",
                format!("Manifest checksum mismatch for {}", file.file_name),
            ));
        }
    }
    Ok(())
}

fn manifest_files(manifest: &ArchiveManifest) -> [&ManifestFile; 6] {
    [
        &manifest.drill_scenarios.data,
        &manifest.drill_scenarios.index,
        &manifest.abstract_action_paths.data,
        &manifest.abstract_action_paths.index,
        &manifest.hand_strategies.data,
        &manifest.hand_strategies.index,
    ]
}

fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf, ToolError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ToolError::invalid_argument("V3 archive output must have a valid final path component")
        })?;
    Ok(path.with_file_name(format!("{file_name}.{suffix}")))
}

fn publish_directory(building: &Path, output: &Path, overwrite: bool) -> Result<(), ToolError> {
    if !output.exists() {
        fs::rename(building, output)?;
        return Ok(());
    }
    if !overwrite {
        return Err(ToolError::invalid_argument("V3 archive output exists"));
    }
    let backup = sibling_path(output, "previous")?;
    remove_dir_if_exists(&backup)?;
    fs::rename(output, &backup)?;
    if let Err(error) = fs::rename(building, output) {
        let _ = fs::rename(&backup, output);
        return Err(error.into());
    }
    fs::remove_dir_all(backup)?;
    Ok(())
}

fn remove_dir_if_exists(path: &Path) -> Result<(), ToolError> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_failure_during_read_back_verification_removes_building_without_publishing() {
        let temp = tempfile::tempdir().unwrap();
        let source_db = temp.path().join("source.db");
        let out_dir = temp.path().join("archive");
        build_source_fixture(&source_db);
        let options = V3ArchiveExportOptions {
            source_db,
            out_dir: out_dir.clone(),
            dimension: DimensionSpec {
                strategy: "default".to_owned(),
                player_count: 6,
                depth_bb: 100,
            },
            metadata_page_target_bytes: 4096,
            overwrite: false,
        };

        let error = export_v3_archive_with_verifier(&options, |building_dir| {
            corrupt_read_back_payload(building_dir);
            verify_v3_archive_before_publish(building_dir)
        })
        .unwrap_err();

        assert_eq!(error.code(), "VERIFY_ERROR");
        assert!(!out_dir.exists());
        assert!(!sibling_path(&out_dir, "building").unwrap().exists());
    }

    fn corrupt_read_back_payload(archive_dir: &Path) {
        let path = archive_dir.join(super::super::format::HAND_STRATEGIES_DATA_FILE_NAME);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 0xff;
        fs::write(path, bytes).unwrap();
    }

    fn build_source_fixture(path: &Path) {
        Connection::open(path, false)
            .unwrap()
            .exec(
                "CREATE TABLE concrete_lines_default_6max_100BB(
                   id INTEGER PRIMARY KEY,
                   abstract_line TEXT NOT NULL,
                   concrete_line TEXT NOT NULL
                 );
                 CREATE TABLE drill_scenario_lines_default(
                   id INTEGER PRIMARY KEY,
                   drill_name TEXT NOT NULL,
                   abstract_line TEXT NOT NULL,
                   player_count INTEGER NOT NULL,
                   depth INTEGER NOT NULL
                 );
                 CREATE TABLE range_data_default_6max_100BB(
                   concrete_line_id INTEGER NOT NULL,
                   hole_cards TEXT NOT NULL,
                   action_name TEXT NOT NULL,
                   action_size REAL NOT NULL,
                   amount_bb REAL NOT NULL,
                   frequency REAL NOT NULL,
                   hand_ev REAL
                 );
                 INSERT INTO concrete_lines_default_6max_100BB VALUES
                   (10, 'A', 'A-1');
                 INSERT INTO drill_scenario_lines_default VALUES
                   (1, 'rfi', 'A', 6, 100);
                 INSERT INTO range_data_default_6max_100BB VALUES
                   (10, 'AA', 'fold', 0.0, 0.0, 0.0, NULL);",
            )
            .unwrap();
    }
}
