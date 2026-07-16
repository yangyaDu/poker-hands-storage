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
use super::verification::{cross_verify_sqlite_v3, V3VerificationOptions};

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
}

pub fn export_v3_archive(
    options: &V3ArchiveExportOptions,
) -> Result<V3ArchiveExportSummary, ToolError> {
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
        V3Archive::open_with_options(
            &building_dir,
            V3ArchiveOpenOptions {
                verify_file_checksums: true,
                ..V3ArchiveOpenOptions::default()
            },
        )?;
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
