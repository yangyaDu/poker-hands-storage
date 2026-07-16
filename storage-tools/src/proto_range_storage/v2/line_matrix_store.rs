use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use memmap2::Mmap;
use prost::Message;
use range_store_core::crc32c::{assert_crc32c, crc32c};
use range_store_core::dimension::{
    discover_dimensions, get_drill_scenario_table_name, quote_identifier, DimensionSpec,
};
use range_store_core::sqlite::{Connection, Value};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::errors::ToolError;
use crate::proto_range_storage::v2::line_matrix_codec::{
    build_compact_index_map, build_compact_line_matrix, count_bits, validate_compact_line_matrix,
    HAND_COUNT_169,
};
use crate::proto_range_storage::v2::proto::{
    ActionType, CompactActionColumn, CompactLineMatrix, HandEncoding,
};
use crate::proto_range_storage::v2::sqlite_source::{load_all_lines, load_rows_with_ev, ResolvedLine};

pub use crate::proto_range_storage::v2::benchmark::{
    run_compact_vs_core_benchmark, run_compact_vs_core_cold_worker, CompactVsCoreBenchmarkCommand,
    CompactVsCoreColdWorkerCommand, CompactVsCoreEngine, CompactVsCoreQuery,
};

use crate::proto_range_storage::v2::format::{
    read_header, read_index_record_from_slice, write_header, write_index_record, IndexRecord,
    DATA_FILE_NAME, DATA_MAGIC, HEADER_SIZE, INDEX_FILE_NAME, INDEX_MAGIC, INDEX_RECORD_SIZE,
    MANIFEST_FILE_NAME, METADATA_FILE_NAME,
};

const ARCHIVE_FORMAT: &str = "LMSP";
const ARCHIVE_VERSION: u32 = 2;
const PAYLOAD_SCHEMA: &str = "zenithstrat.gto.v2.CompactLineMatrix";
pub const DEFAULT_MATRIX_CACHE_CAPACITY: usize = 1024;

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

#[derive(Debug, Clone)]
pub struct CompactLineMatrixArchiveOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub overwrite: bool,
}

#[derive(Debug, Clone)]
pub struct CompactLineMatrixArchiveSummary {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub matrix_count: u64,
    pub action_value_count: u64,
    pub protobuf_bytes: u64,
    pub manifest_path: PathBuf,
    pub data_path: PathBuf,
    pub index_path: PathBuf,
    pub metadata_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactLineMatrixArchiveVerificationSummary {
    pub matrix_count: u64,
    pub action_count: u64,
    pub action_value_count: u64,
}

#[derive(Debug, Clone)]
pub struct CompactLineMatrixArchivesOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactLineMatrixDimensionStorageSummary {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
    pub matrix_count: u64,
    pub action_value_count: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub bin_idx_bytes: u64,
    pub metadata_bytes: u64,
    pub manifest_bytes: u64,
    pub archive_bytes: u64,
    pub archive_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactLineMatrixStorageReport {
    pub source_db: PathBuf,
    pub sqlite_bytes: u64,
    pub dimensions: Vec<CompactLineMatrixDimensionStorageSummary>,
    pub total_data_bytes: u64,
    pub total_index_bytes: u64,
    pub total_bin_idx_bytes: u64,
    pub total_archive_bytes: u64,
    pub bin_idx_to_sqlite_ratio: f64,
    pub bin_idx_to_sqlite_percent: f64,
    pub sqlite_share_percent: f64,
    pub bin_idx_share_percent: f64,
    #[serde(skip)]
    pub report_path: PathBuf,
}

#[derive(Debug)]
pub struct CompactLineMatrixArchive {
    data_mmap: Mmap,
    index_mmap: Mmap,
    _data_file: File,
    _index_file: File,
    dimension: DimensionSpec,
    matrix_count: u64,
    verify_checksums: bool,
    cache: Mutex<SimpleLru>,
}

#[derive(Debug, Clone)]
pub struct CompactArchiveOpenOptions {
    pub verify_checksums: bool,
    pub cache_capacity: usize,
    pub cache_byte_budget: Option<usize>,
}

impl Default for CompactArchiveOpenOptions {
    fn default() -> Self {
        Self {
            verify_checksums: true,
            cache_capacity: DEFAULT_MATRIX_CACHE_CAPACITY,
            cache_byte_budget: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandActionValue {
    pub frequency_x10000: u32,
    pub ev_x10000: i32,
}

#[derive(Debug, Clone)]
pub struct DecodedCompactLineMatrix {
    matrix: CompactLineMatrix,
    hand_id_to_global_index: Vec<i16>,
    action_offsets: Vec<usize>,
    action_global_to_local_index: Vec<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatrixCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub entries: usize,
    pub resident_estimated_bytes: usize,
    pub peak_resident_estimated_bytes: usize,
    pub evictions: u64,
    pub evicted_estimated_bytes: u64,
    pub oversized_skips: u64,
    pub cache_disabled_skips: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCacheInsertOutcome {
    NotAttempted,
    Cached,
    Disabled,
    Oversized,
}

#[derive(Debug, Clone, Copy)]
pub struct MatrixReadPhaseProfile {
    pub cache_hit: bool,
    pub cache_insert_outcome: MatrixCacheInsertOutcome,
    pub cache_lookup_ms: f64,
    pub index_payload_ms: f64,
    pub protobuf_decode_ms: f64,
    pub compact_index_ms: f64,
    pub cache_insert_ms: f64,
    pub total_ms: f64,
}

#[derive(Debug, Clone)]
pub struct ProfiledMatrixRead {
    pub matrix: Arc<DecodedCompactLineMatrix>,
    pub profile: MatrixReadPhaseProfile,
}

#[derive(Debug)]
struct SimpleLru {
    capacity: usize,
    byte_budget: Option<usize>,
    data: std::collections::HashMap<u64, CachedMatrix>,
    counter: u64,
    hits: u64,
    misses: u64,
    resident_estimated_bytes: usize,
    peak_resident_estimated_bytes: usize,
    evictions: u64,
    evicted_estimated_bytes: u64,
    oversized_skips: u64,
    cache_disabled_skips: u64,
}

#[derive(Debug)]
struct CachedMatrix {
    value: Arc<DecodedCompactLineMatrix>,
    last_access: u64,
    estimated_bytes: usize,
}

impl SimpleLru {
    fn new(capacity: usize, byte_budget: Option<usize>) -> Self {
        Self {
            capacity,
            byte_budget,
            data: std::collections::HashMap::new(),
            counter: 0,
            hits: 0,
            misses: 0,
            resident_estimated_bytes: 0,
            peak_resident_estimated_bytes: 0,
            evictions: 0,
            evicted_estimated_bytes: 0,
            oversized_skips: 0,
            cache_disabled_skips: 0,
        }
    }

    fn get(&mut self, key: u64) -> Option<Arc<DecodedCompactLineMatrix>> {
        let entry = self.data.get_mut(&key)?;
        self.counter = self.counter.wrapping_add(1);
        entry.last_access = self.counter;
        self.hits = self.hits.wrapping_add(1);
        Some(Arc::clone(&entry.value))
    }

    fn record_miss(&mut self) {
        self.misses = self.misses.wrapping_add(1);
    }

    fn put(&mut self, key: u64, value: Arc<DecodedCompactLineMatrix>) -> MatrixCacheInsertOutcome {
        if self.capacity == 0 {
            self.cache_disabled_skips = self.cache_disabled_skips.wrapping_add(1);
            return MatrixCacheInsertOutcome::Disabled;
        }
        let estimated_bytes = value.estimated_heap_bytes();
        if self
            .byte_budget
            .is_some_and(|budget| estimated_bytes > budget)
        {
            self.oversized_skips = self.oversized_skips.wrapping_add(1);
            return MatrixCacheInsertOutcome::Oversized;
        }
        self.counter = self.counter.wrapping_add(1);
        if let Some(previous) = self.data.remove(&key) {
            self.resident_estimated_bytes = self
                .resident_estimated_bytes
                .saturating_sub(previous.estimated_bytes);
        }
        while self.data.len() >= self.capacity
            || self.byte_budget.is_some_and(|budget| {
                self.resident_estimated_bytes
                    .saturating_add(estimated_bytes)
                    > budget
            })
        {
            let Some(lru_key) = self
                .data
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| *key)
            else {
                self.oversized_skips = self.oversized_skips.wrapping_add(1);
                return MatrixCacheInsertOutcome::Oversized;
            };
            if let Some(evicted) = self.data.remove(&lru_key) {
                self.resident_estimated_bytes = self
                    .resident_estimated_bytes
                    .saturating_sub(evicted.estimated_bytes);
                self.evictions = self.evictions.wrapping_add(1);
                self.evicted_estimated_bytes = self
                    .evicted_estimated_bytes
                    .wrapping_add(evicted.estimated_bytes as u64);
            }
        }
        self.resident_estimated_bytes = self
            .resident_estimated_bytes
            .saturating_add(estimated_bytes);
        self.peak_resident_estimated_bytes = self
            .peak_resident_estimated_bytes
            .max(self.resident_estimated_bytes);
        self.data.insert(
            key,
            CachedMatrix {
                value,
                last_access: self.counter,
                estimated_bytes,
            },
        );
        MatrixCacheInsertOutcome::Cached
    }

    fn stats(&self) -> MatrixCacheStats {
        MatrixCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.data.len(),
            resident_estimated_bytes: self.resident_estimated_bytes,
            peak_resident_estimated_bytes: self.peak_resident_estimated_bytes,
            evictions: self.evictions,
            evicted_estimated_bytes: self.evicted_estimated_bytes,
            oversized_skips: self.oversized_skips,
            cache_disabled_skips: self.cache_disabled_skips,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveManifest {
    format: String,
    version: u32,
    payload_schema: String,
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    matrix_schema_version: u32,
    hand_encoding: String,
    matrix_count: u64,
    data_file: String,
    index_file: String,
    metadata_file: String,
    data_file_size_bytes: u64,
    index_file_size_bytes: u64,
    metadata_file_size_bytes: u64,
}

pub fn export_compact_line_matrix_archive(
    options: &CompactLineMatrixArchiveOptions,
) -> Result<CompactLineMatrixArchiveSummary, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    let dimension = options.dimension.clone();
    let source = Connection::open(&options.source_db, true)?;
    let lines = load_all_lines(&source, &dimension)?;
    validate_dense_line_ids(&lines)?;
    prepare_output_dir(&options.out_dir, options.overwrite)?;

    let data_path = options.out_dir.join(DATA_FILE_NAME);
    let index_path = options.out_dir.join(INDEX_FILE_NAME);
    let metadata_path = options.out_dir.join(METADATA_FILE_NAME);
    let manifest_path = options.out_dir.join(MANIFEST_FILE_NAME);
    let data_tmp = data_path.with_extension("lmbin.tmp");
    let index_tmp = index_path.with_extension("lmidx.tmp");
    let metadata_tmp = metadata_path.with_extension("db.tmp");
    let manifest_tmp = manifest_path.with_extension("json.tmp");
    let temporary_paths = [&data_tmp, &index_tmp, &metadata_tmp, &manifest_tmp];
    for path in temporary_paths {
        remove_if_exists(path)?;
    }

    let (matrix_count, protobuf_bytes, action_value_count) = match build_archive_files(
        &source,
        &dimension,
        &lines,
        &data_tmp,
        &index_tmp,
        &metadata_tmp,
    ) {
        Ok(summary) => summary,
        Err(error) => {
            for path in temporary_paths {
                let _ = fs::remove_file(path);
            }
            return Err(error);
        }
    };
    let manifest = ArchiveManifest {
        format: ARCHIVE_FORMAT.to_owned(),
        version: ARCHIVE_VERSION,
        payload_schema: PAYLOAD_SCHEMA.to_owned(),
        strategy: dimension.strategy.clone(),
        player_count: dimension.player_count,
        depth_bb: dimension.depth_bb,
        matrix_schema_version: 2,
        hand_encoding: HandEncoding::HandEncoding169.as_str_name().to_owned(),
        matrix_count,
        data_file: DATA_FILE_NAME.to_owned(),
        index_file: INDEX_FILE_NAME.to_owned(),
        metadata_file: METADATA_FILE_NAME.to_owned(),
        data_file_size_bytes: fs::metadata(&data_tmp)?.len(),
        index_file_size_bytes: fs::metadata(&index_tmp)?.len(),
        metadata_file_size_bytes: fs::metadata(&metadata_tmp)?.len(),
    };
    write_manifest(&manifest_tmp, &manifest)?;
    for path in [&data_path, &index_path, &metadata_path, &manifest_path] {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    fs::rename(&data_tmp, &data_path)?;
    fs::rename(&index_tmp, &index_path)?;
    fs::rename(&metadata_tmp, &metadata_path)?;
    fs::rename(&manifest_tmp, &manifest_path)?;
    Ok(CompactLineMatrixArchiveSummary {
        strategy: dimension.strategy,
        player_count: dimension.player_count,
        depth_bb: dimension.depth_bb,
        matrix_count,
        action_value_count,
        protobuf_bytes,
        manifest_path,
        data_path,
        index_path,
        metadata_path,
    })
}

pub fn export_all_compact_line_matrix_archives(
    options: &CompactLineMatrixArchivesOptions,
) -> Result<CompactLineMatrixStorageReport, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    fs::create_dir_all(&options.out_dir)?;
    let report_path = options.out_dir.join("storage-comparison.json");
    if report_path.exists() && !options.overwrite {
        return Err(ToolError::invalid_argument(format!(
            "Storage report already exists: {}. Use --overwrite to replace it",
            report_path.display()
        )));
    }

    let sqlite_bytes = fs::metadata(&options.source_db)?.len();
    let source = Connection::open(&options.source_db, true)?;
    let dimensions = discover_dimensions(&source)?;
    drop(source);
    if dimensions.is_empty() {
        return Err(ToolError::new(
            "LINE_MATRIX_ARCHIVE_EMPTY",
            "Source database has no discoverable range dimensions",
        ));
    }

    let mut summaries = Vec::with_capacity(dimensions.len());
    let mut total_data_bytes = 0u64;
    let mut total_index_bytes = 0u64;
    let mut total_archive_bytes = 0u64;
    for dimension in dimensions {
        let archive_dir = options.out_dir.join(format!(
            "{}_{}max_{}BB",
            dimension.strategy, dimension.player_count, dimension.depth_bb
        ));
        let summary = export_compact_line_matrix_archive(&CompactLineMatrixArchiveOptions {
            source_db: options.source_db.clone(),
            out_dir: archive_dir.clone(),
            dimension: dimension.clone(),
            overwrite: options.overwrite,
        })?;
        let verification = CompactLineMatrixArchive::open(&archive_dir)?.verify_all()?;
        if verification.matrix_count != summary.matrix_count
            || verification.action_value_count != summary.action_value_count
        {
            return Err(ToolError::new(
                "COMPACT_ARCHIVE_VERIFICATION_MISMATCH",
                format!(
                    "Dimension {}:{}:{} export and read-back totals differ",
                    dimension.strategy, dimension.player_count, dimension.depth_bb
                ),
            ));
        }

        let data_bytes = fs::metadata(&summary.data_path)?.len();
        let index_bytes = fs::metadata(&summary.index_path)?.len();
        let metadata_bytes = fs::metadata(&summary.metadata_path)?.len();
        let manifest_bytes = fs::metadata(&summary.manifest_path)?.len();
        let bin_idx_bytes = data_bytes
            .checked_add(index_bytes)
            .ok_or_else(|| ToolError::invalid_format("Compact bin+idx size overflow"))?;
        let archive_bytes = bin_idx_bytes
            .checked_add(metadata_bytes)
            .and_then(|size| size.checked_add(manifest_bytes))
            .ok_or_else(|| ToolError::invalid_format("Compact archive size overflow"))?;
        total_data_bytes = total_data_bytes
            .checked_add(data_bytes)
            .ok_or_else(|| ToolError::invalid_format("Compact data size overflow"))?;
        total_index_bytes = total_index_bytes
            .checked_add(index_bytes)
            .ok_or_else(|| ToolError::invalid_format("Compact index size overflow"))?;
        total_archive_bytes = total_archive_bytes
            .checked_add(archive_bytes)
            .ok_or_else(|| ToolError::invalid_format("Compact archive size overflow"))?;
        summaries.push(CompactLineMatrixDimensionStorageSummary {
            strategy: dimension.strategy,
            player_count: dimension.player_count,
            depth_bb: dimension.depth_bb,
            matrix_count: summary.matrix_count,
            action_value_count: summary.action_value_count,
            data_bytes,
            index_bytes,
            bin_idx_bytes,
            metadata_bytes,
            manifest_bytes,
            archive_bytes,
            archive_dir,
        });
    }

    let total_bin_idx_bytes = total_data_bytes
        .checked_add(total_index_bytes)
        .ok_or_else(|| ToolError::invalid_format("Compact bin+idx total size overflow"))?;
    let comparison_total = sqlite_bytes
        .checked_add(total_bin_idx_bytes)
        .ok_or_else(|| ToolError::invalid_format("Storage comparison size overflow"))?;
    let bin_idx_to_sqlite_ratio = total_bin_idx_bytes as f64 / sqlite_bytes as f64;
    let report = CompactLineMatrixStorageReport {
        source_db: options.source_db.clone(),
        sqlite_bytes,
        dimensions: summaries,
        total_data_bytes,
        total_index_bytes,
        total_bin_idx_bytes,
        total_archive_bytes,
        bin_idx_to_sqlite_ratio,
        bin_idx_to_sqlite_percent: bin_idx_to_sqlite_ratio * 100.0,
        sqlite_share_percent: sqlite_bytes as f64 / comparison_total as f64 * 100.0,
        bin_idx_share_percent: total_bin_idx_bytes as f64 / comparison_total as f64 * 100.0,
        report_path: report_path.clone(),
    };
    let json = serde_json::to_string_pretty(&report)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(&report_path, format!("{json}\n"))?;
    Ok(report)
}

impl CompactLineMatrixArchiveVerificationSummary {
    fn empty() -> Self {
        Self {
            matrix_count: 0,
            action_count: 0,
            action_value_count: 0,
        }
    }

    fn include(&mut self, decoded: &DecodedCompactLineMatrix) -> Result<(), ToolError> {
        self.matrix_count = self
            .matrix_count
            .checked_add(1)
            .ok_or_else(|| ToolError::invalid_format("Compact archive matrix count overflow"))?;
        self.action_count = self
            .action_count
            .checked_add(decoded.matrix.actions.len() as u64)
            .ok_or_else(|| ToolError::invalid_format("Compact archive action count overflow"))?;
        for action in &decoded.matrix.actions {
            self.action_value_count = self
                .action_value_count
                .checked_add(action.frequency_x10000.len() as u64)
                .ok_or_else(|| {
                    ToolError::invalid_format("Compact archive action value count overflow")
                })?;
        }
        Ok(())
    }

    fn merge(&mut self, other: Self) -> Result<(), ToolError> {
        self.matrix_count = self
            .matrix_count
            .checked_add(other.matrix_count)
            .ok_or_else(|| ToolError::invalid_format("Compact archive matrix count overflow"))?;
        self.action_count = self
            .action_count
            .checked_add(other.action_count)
            .ok_or_else(|| ToolError::invalid_format("Compact archive action count overflow"))?;
        self.action_value_count = self
            .action_value_count
            .checked_add(other.action_value_count)
            .ok_or_else(|| {
                ToolError::invalid_format("Compact archive action value count overflow")
            })?;
        Ok(())
    }
}

impl CompactLineMatrixArchive {
    pub fn open(dir: &Path) -> Result<Self, ToolError> {
        Self::open_with_options(dir, CompactArchiveOpenOptions::default())
    }

    pub fn open_with_options(
        dir: &Path,
        options: CompactArchiveOpenOptions,
    ) -> Result<Self, ToolError> {
        let manifest = read_archive_manifest(dir)?;
        let data_path = dir.join(&manifest.data_file);
        let index_path = dir.join(&manifest.index_file);
        if !dir.join(&manifest.metadata_file).is_file() {
            return Err(ToolError::invalid_format(
                "Compact archive metadata file does not exist",
            ));
        }

        let mut data_file = File::open(&data_path)?;
        let mut index_file = File::open(&index_path)?;
        let data_count = read_header(&mut data_file, DATA_MAGIC)?;
        let index_count = read_header(&mut index_file, INDEX_MAGIC)?;
        if data_count != manifest.matrix_count || index_count != manifest.matrix_count {
            return Err(ToolError::invalid_format(
                "Compact archive record counts differ between manifest and binary files",
            ));
        }
        let expected_index_size = (HEADER_SIZE as u64)
            .checked_add(
                manifest
                    .matrix_count
                    .checked_mul(INDEX_RECORD_SIZE as u64)
                    .ok_or_else(|| {
                        ToolError::invalid_format("Compact archive index size overflow")
                    })?,
            )
            .ok_or_else(|| ToolError::invalid_format("Compact archive index size overflow"))?;
        let data_size = data_file.metadata()?.len();
        let index_size = index_file.metadata()?.len();
        if index_size != expected_index_size || index_size != manifest.index_file_size_bytes {
            return Err(ToolError::invalid_format(
                "Compact archive index file size is invalid",
            ));
        }
        if data_size != manifest.data_file_size_bytes {
            return Err(ToolError::invalid_format(
                "Compact archive data file size is invalid",
            ));
        }

        // SAFETY: archive files are opened read-only, retained for the mapping lifetime,
        // and must remain immutable while this reader is alive.
        let data_mmap = unsafe { Mmap::map(&data_file)? };
        // SAFETY: same immutable archive contract as the data mapping above.
        let index_mmap = unsafe { Mmap::map(&index_file)? };
        Ok(Self {
            data_mmap,
            index_mmap,
            _data_file: data_file,
            _index_file: index_file,
            dimension: DimensionSpec {
                strategy: manifest.strategy,
                player_count: manifest.player_count,
                depth_bb: manifest.depth_bb,
            },
            matrix_count: manifest.matrix_count,
            verify_checksums: options.verify_checksums,
            cache: Mutex::new(SimpleLru::new(
                options.cache_capacity,
                options.cache_byte_budget,
            )),
        })
    }

    pub fn dimension(&self) -> &DimensionSpec {
        &self.dimension
    }

    pub fn matrix_count(&self) -> u64 {
        self.matrix_count
    }

    pub fn matrix_cache_stats(&self) -> MatrixCacheStats {
        self.cache
            .lock()
            .expect("compact cache lock poisoned")
            .stats()
    }

    pub fn read_matrix(
        &self,
        concrete_line_id: u64,
    ) -> Result<Arc<DecodedCompactLineMatrix>, ToolError> {
        Ok(self.read_matrix_profiled(concrete_line_id)?.matrix)
    }

    pub fn read_matrix_profiled(
        &self,
        concrete_line_id: u64,
    ) -> Result<ProfiledMatrixRead, ToolError> {
        let total_started = Instant::now();
        let stage_started = Instant::now();
        {
            let mut cache = self.cache.lock().expect("compact cache lock poisoned");
            if let Some(decoded) = cache.get(concrete_line_id) {
                return Ok(ProfiledMatrixRead {
                    matrix: decoded,
                    profile: MatrixReadPhaseProfile {
                        cache_hit: true,
                        cache_insert_outcome: MatrixCacheInsertOutcome::NotAttempted,
                        cache_lookup_ms: elapsed_ms(stage_started),
                        index_payload_ms: 0.0,
                        protobuf_decode_ms: 0.0,
                        compact_index_ms: 0.0,
                        cache_insert_ms: 0.0,
                        total_ms: elapsed_ms(total_started),
                    },
                });
            }
            cache.record_miss();
        }
        let cache_lookup_ms = elapsed_ms(stage_started);
        let (decoded, index_payload_ms, protobuf_decode_ms, compact_index_ms) =
            self.decode_matrix_uncached_profiled(concrete_line_id, self.verify_checksums)?;
        let decoded = Arc::new(decoded);
        let stage_started = Instant::now();
        let cache_insert_outcome = {
            let mut cache = self.cache.lock().expect("compact cache lock poisoned");
            cache.put(concrete_line_id, Arc::clone(&decoded))
        };
        Ok(ProfiledMatrixRead {
            matrix: decoded,
            profile: MatrixReadPhaseProfile {
                cache_hit: false,
                cache_insert_outcome,
                cache_lookup_ms,
                index_payload_ms,
                protobuf_decode_ms,
                compact_index_ms,
                cache_insert_ms: elapsed_ms(stage_started),
                total_ms: elapsed_ms(total_started),
            },
        })
    }

    fn decode_matrix_uncached(
        &self,
        concrete_line_id: u64,
        verify: bool,
    ) -> Result<DecodedCompactLineMatrix, ToolError> {
        Ok(self
            .decode_matrix_uncached_profiled(concrete_line_id, verify)?
            .0)
    }

    fn decode_matrix_uncached_profiled(
        &self,
        concrete_line_id: u64,
        verify: bool,
    ) -> Result<(DecodedCompactLineMatrix, f64, f64, f64), ToolError> {
        let stage_started = Instant::now();
        if concrete_line_id == 0 || concrete_line_id > self.matrix_count {
            return Err(ToolError::new(
                "LINE_NOT_FOUND",
                format!("Concrete line {concrete_line_id} is not in this archive"),
            ));
        }
        let record = read_index_record_from_slice(&self.index_mmap, concrete_line_id)?;
        let payload_end = record
            .offset
            .checked_add(u64::from(record.byte_length))
            .ok_or_else(|| ToolError::invalid_format("Compact archive payload offset overflow"))?;
        if record.offset < HEADER_SIZE as u64 || payload_end > self.data_mmap.len() as u64 {
            return Err(ToolError::invalid_format(
                "Compact archive index record points outside data file",
            ));
        }
        let start = usize::try_from(record.offset)
            .map_err(|_| ToolError::invalid_format("Compact payload offset exceeds usize"))?;
        let end = usize::try_from(payload_end)
            .map_err(|_| ToolError::invalid_format("Compact payload end exceeds usize"))?;
        let payload = self
            .data_mmap
            .get(start..end)
            .ok_or_else(|| ToolError::invalid_format("Compact payload is truncated"))?;
        if verify {
            assert_crc32c(payload, record.crc32c).map_err(ToolError::invalid_format)?;
        }
        let index_payload_ms = elapsed_ms(stage_started);
        let stage_started = Instant::now();
        let matrix = CompactLineMatrix::decode(payload)
            .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
        let protobuf_decode_ms = elapsed_ms(stage_started);
        let stage_started = Instant::now();
        let decoded = DecodedCompactLineMatrix::new(matrix)?;
        Ok((
            decoded,
            index_payload_ms,
            protobuf_decode_ms,
            elapsed_ms(stage_started),
        ))
    }

    pub fn verify_all(&self) -> Result<CompactLineMatrixArchiveVerificationSummary, ToolError> {
        (1..=self.matrix_count)
            .into_par_iter()
            .try_fold(
                CompactLineMatrixArchiveVerificationSummary::empty,
                |mut summary, concrete_line_id| {
                    let decoded = self.decode_matrix_uncached(concrete_line_id, true)?;
                    summary.include(&decoded)?;
                    Ok(summary)
                },
            )
            .try_reduce(
                CompactLineMatrixArchiveVerificationSummary::empty,
                |mut left, right| {
                    left.merge(right)?;
                    Ok(left)
                },
            )
    }

    pub fn verify_all_sequential(
        &self,
    ) -> Result<CompactLineMatrixArchiveVerificationSummary, ToolError> {
        let mut summary = CompactLineMatrixArchiveVerificationSummary::empty();
        for concrete_line_id in 1..=self.matrix_count {
            let decoded = self.decode_matrix_uncached(concrete_line_id, true)?;
            summary.include(&decoded)?;
        }
        Ok(summary)
    }
}

pub fn read_compact_archive_dimension(dir: &Path) -> Result<DimensionSpec, ToolError> {
    let manifest = read_archive_manifest(dir)?;
    Ok(DimensionSpec {
        strategy: manifest.strategy,
        player_count: manifest.player_count,
        depth_bb: manifest.depth_bb,
    })
}

fn read_archive_manifest(dir: &Path) -> Result<ArchiveManifest, ToolError> {
    let manifest_path = dir.join(MANIFEST_FILE_NAME);
    let manifest: ArchiveManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

impl DecodedCompactLineMatrix {
    fn new(matrix: CompactLineMatrix) -> Result<Self, ToolError> {
        validate_compact_line_matrix(&matrix)?;
        let hand_id_to_global_index =
            build_compact_index_map(&matrix.valid_hand_bitmap, HAND_COUNT_169);
        let valid_hand_count = count_bits(&matrix.valid_hand_bitmap);
        let mut action_offsets = Vec::with_capacity(matrix.actions.len() + 1);
        let mut action_global_to_local_index =
            Vec::with_capacity(matrix.actions.len() * valid_hand_count);
        for action in &matrix.actions {
            action_offsets.push(action_global_to_local_index.len());
            action_global_to_local_index.extend(build_compact_index_map(
                &action.action_hand_bitmap,
                valid_hand_count,
            ));
        }
        action_offsets.push(action_global_to_local_index.len());
        Ok(Self {
            matrix,
            hand_id_to_global_index,
            action_offsets,
            action_global_to_local_index,
        })
    }

    pub fn matrix(&self) -> &CompactLineMatrix {
        &self.matrix
    }

    pub fn estimated_heap_bytes(&self) -> usize {
        let action_payload_bytes = self
            .matrix
            .actions
            .iter()
            .map(|action| {
                action.frequency_x10000.capacity() * std::mem::size_of::<u32>()
                    + action.ev_x10000.capacity() * std::mem::size_of::<i32>()
                    + action.action_hand_bitmap.capacity()
            })
            .sum::<usize>();
        std::mem::size_of::<Self>()
            + self.matrix.actions.capacity() * std::mem::size_of::<CompactActionColumn>()
            + self.matrix.valid_hand_bitmap.capacity()
            + action_payload_bytes
            + self.hand_id_to_global_index.capacity() * std::mem::size_of::<i16>()
            + self.action_offsets.capacity() * std::mem::size_of::<usize>()
            + self.action_global_to_local_index.capacity() * std::mem::size_of::<i16>()
    }

    pub fn action_value(&self, action_index: usize, hand_id: usize) -> Option<HandActionValue> {
        let global_index = *self.hand_id_to_global_index.get(hand_id)?;
        if global_index < 0 {
            return None;
        }
        let action = self.matrix.actions.get(action_index)?;
        let start = *self.action_offsets.get(action_index)?;
        let end = *self.action_offsets.get(action_index + 1)?;
        let position = start.checked_add(global_index as usize)?;
        if position >= end {
            return None;
        }
        let local_index = *self.action_global_to_local_index.get(position)?;
        if local_index < 0 {
            return None;
        }
        Some(HandActionValue {
            frequency_x10000: *action.frequency_x10000.get(local_index as usize)?,
            ev_x10000: *action.ev_x10000.get(local_index as usize)?,
        })
    }

    pub fn action_value_by_identity(
        &self,
        action_type: ActionType,
        action_size_x10000: u32,
        amount_centi_bb: u32,
        hand_id: usize,
    ) -> Option<HandActionValue> {
        let action_index = self.matrix.actions.iter().position(|action| {
            action.action_type == action_type as i32
                && action.action_size_x10000 == action_size_x10000
                && action.amount_centi_bb == amount_centi_bb
        })?;
        self.action_value(action_index, hand_id)
    }
}

fn build_archive_files(
    source: &Connection,
    dimension: &DimensionSpec,
    lines: &[ResolvedLine],
    data_tmp: &Path,
    index_tmp: &Path,
    metadata_tmp: &Path,
) -> Result<(u64, u64, u64), ToolError> {
    let mut data = create_new_file(data_tmp)?;
    let mut index = create_new_file(index_tmp)?;
    write_header(&mut data, DATA_MAGIC, 0)?;
    write_header(&mut index, INDEX_MAGIC, 0)?;
    let metadata = Connection::open(metadata_tmp, false)?;
    init_metadata_db(&metadata, &dimension.strategy)?;
    metadata.exec("BEGIN")?;
    let result = (|| {
        copy_drill_scenario_lines(source, &metadata, &dimension.strategy)?;
        let mut offset = HEADER_SIZE as u64;
        let mut protobuf_bytes = 0u64;
        let mut action_value_count = 0u64;
        for line in lines {
            let rows = load_rows_with_ev(source, dimension, line.concrete_line_id)?;
            let matrix = build_compact_line_matrix(&rows)?;
            let matrix_action_value_count = matrix
                .actions
                .iter()
                .map(|action| action.frequency_x10000.len())
                .sum::<usize>();
            if matrix_action_value_count != rows.len() {
                return Err(ToolError::new(
                    "COMPACT_ACTION_VALUE_COUNT_MISMATCH",
                    format!(
                        "Concrete line {} encoded {matrix_action_value_count} values from {} non-NULL EV rows",
                        line.concrete_line_id,
                        rows.len()
                    ),
                ));
            }
            action_value_count = action_value_count
                .checked_add(matrix_action_value_count as u64)
                .ok_or_else(|| {
                    ToolError::invalid_format("Compact archive action value count overflow")
                })?;
            let payload = matrix.encode_to_vec();
            let decoded = CompactLineMatrix::decode(payload.as_slice())
                .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
            validate_compact_line_matrix(&decoded)?;
            if decoded != matrix {
                return Err(ToolError::new(
                    "PROTOBUF_ROUNDTRIP_MISMATCH",
                    "Decoded CompactLineMatrix differs from the encoded matrix",
                ));
            }
            let byte_length = u32::try_from(payload.len()).map_err(|_| {
                ToolError::invalid_format("CompactLineMatrix payload exceeds u32 length limit")
            })?;
            data.write_all(&payload)?;
            write_index_record(
                &mut index,
                IndexRecord {
                    offset,
                    byte_length,
                    crc32c: crc32c(&payload),
                },
            )?;
            metadata.execute(
                "INSERT INTO concrete_lines(concrete_line_id, abstract_line, concrete_line)
                 VALUES (?1, ?2, ?3)",
                &[
                    Value::from(line.concrete_line_id),
                    Value::from(line.abstract_line.as_str()),
                    Value::from(line.concrete_line.as_str()),
                ],
            )?;
            offset = offset
                .checked_add(u64::from(byte_length))
                .ok_or_else(|| ToolError::invalid_format("Compact archive data offset overflow"))?;
            protobuf_bytes = protobuf_bytes
                .checked_add(u64::from(byte_length))
                .ok_or_else(|| {
                    ToolError::invalid_format("Compact archive payload size overflow")
                })?;
        }
        let matrix_count = u64::try_from(lines.len())
            .map_err(|_| ToolError::invalid_format("Too many compact matrices"))?;
        metadata.exec("COMMIT")?;
        write_header(&mut data, DATA_MAGIC, matrix_count)?;
        write_header(&mut index, INDEX_MAGIC, matrix_count)?;
        data.sync_all()?;
        index.sync_all()?;
        Ok((matrix_count, protobuf_bytes, action_value_count))
    })();
    if result.is_err() {
        let _ = metadata.exec("ROLLBACK");
    }
    drop(metadata);
    result
}

fn validate_dense_line_ids(lines: &[ResolvedLine]) -> Result<(), ToolError> {
    for (index, line) in lines.iter().enumerate() {
        let expected = u32::try_from(index + 1)
            .map_err(|_| ToolError::invalid_format("Too many concrete line ids"))?;
        if line.concrete_line_id != expected {
            return Err(ToolError::new(
                "NON_DENSE_CONCRETE_LINE_IDS",
                format!(
                    "Expected concrete_line_id={expected}, got {}",
                    line.concrete_line_id
                ),
            ));
        }
    }
    Ok(())
}

fn prepare_output_dir(out_dir: &Path, overwrite: bool) -> Result<(), ToolError> {
    fs::create_dir_all(out_dir)?;
    let artifacts = [
        out_dir.join(DATA_FILE_NAME),
        out_dir.join(INDEX_FILE_NAME),
        out_dir.join(METADATA_FILE_NAME),
        out_dir.join(MANIFEST_FILE_NAME),
    ];
    if !overwrite {
        if let Some(existing) = artifacts.iter().find(|path| path.exists()) {
            return Err(ToolError::invalid_argument(format!(
                "Archive output already exists: {}. Use --overwrite to replace it",
                existing.display()
            )));
        }
    }
    Ok(())
}

fn init_metadata_db(connection: &Connection, strategy: &str) -> Result<(), ToolError> {
    let drill_table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
    connection.exec(&format!(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE concrete_lines (
           concrete_line_id INTEGER PRIMARY KEY,
           abstract_line TEXT NOT NULL,
           concrete_line TEXT NOT NULL,
           UNIQUE(abstract_line, concrete_line)
         );
         CREATE INDEX idx_concrete_lines_concrete_line ON concrete_lines(concrete_line);
         CREATE TABLE {drill_table} (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           drill_name TEXT NOT NULL,
           abstract_line TEXT NOT NULL,
           player_count INTEGER NOT NULL,
           drill_depth INTEGER NOT NULL DEFAULT 100,
           UNIQUE(drill_name, player_count, drill_depth, abstract_line)
         );"
    ))?;
    Ok(())
}

fn copy_drill_scenario_lines(
    source: &Connection,
    target: &Connection,
    strategy: &str,
) -> Result<(), ToolError> {
    let raw_table = get_drill_scenario_table_name(strategy);
    let mut exists = source.prepare(
        "SELECT EXISTS(
           SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
         )",
    )?;
    exists.start(&[Value::from(raw_table.as_str())])?;
    if !exists.step_row()? || exists.column_i64(0) == 0 {
        return Ok(());
    }

    let table = quote_identifier(&raw_table)?;
    let mut select = source.prepare(&format!(
        "SELECT drill_name, abstract_line, player_count, depth
         FROM {table}
         ORDER BY id"
    ))?;
    select.start(&[])?;
    let mut insert = target.prepare(&format!(
        "INSERT OR IGNORE INTO {table}(
           drill_name, abstract_line, player_count, drill_depth
         ) VALUES (?1, ?2, ?3, ?4)"
    ))?;
    while select.step_row()? {
        insert.execute(&[
            Value::from(select.column_text(0)?),
            Value::from(select.column_text(1)?),
            Value::from(select.column_u32(2)?),
            Value::from(select.column_u32(3)?),
        ])?;
    }
    Ok(())
}

fn write_manifest(path: &Path, manifest: &ArchiveManifest) -> Result<(), ToolError> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|error| ToolError::invalid_format(error.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

fn validate_manifest(manifest: &ArchiveManifest) -> Result<(), ToolError> {
    if manifest.format != ARCHIVE_FORMAT
        || manifest.version != ARCHIVE_VERSION
        || manifest.payload_schema != PAYLOAD_SCHEMA
    {
        return Err(ToolError::invalid_format(
            "Unsupported Compact LineMatrix archive manifest",
        ));
    }
    if manifest.strategy.is_empty()
        || manifest.player_count == 0
        || manifest.depth_bb == 0
        || manifest.matrix_schema_version != 2
        || manifest.hand_encoding != HandEncoding::HandEncoding169.as_str_name()
    {
        return Err(ToolError::invalid_format(
            "Unsupported Compact LineMatrix payload schema",
        ));
    }
    if manifest.data_file != DATA_FILE_NAME
        || manifest.index_file != INDEX_FILE_NAME
        || manifest.metadata_file != METADATA_FILE_NAME
    {
        return Err(ToolError::invalid_format(
            "Compact archive file names are invalid",
        ));
    }
    Ok(())
}

fn create_new_file(path: &Path) -> Result<File, ToolError> {
    OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(path)
        .map_err(ToolError::from)
}

fn remove_if_exists(path: &Path) -> Result<(), ToolError> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
