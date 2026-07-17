use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use prost::Message;
use range_store_core::crc32c::assert_crc32c;
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

use super::cache::{ByteCacheStats, ByteLru};
use super::format::{
    decode_header, decode_payload_locator, decode_section_descriptor, FileKind, PayloadLocator,
    SectionKind, HAND_STRATEGIES_DATA_FILE_NAME, HAND_STRATEGIES_INDEX_FILE_NAME, HEADER_SIZE,
    PAYLOAD_LOCATOR_SIZE, SECTION_DESCRIPTOR_SIZE,
};
use super::manifest::HandStrategiesManifest;
use super::metadata_store::ExportedConcreteActionPath;
use super::proto::HandStrategy;
use super::source::load_strategy_rows;
use super::storage_file::{
    manifest_file, payload_locator_section, write_index_file, write_payload_data_file,
    StagedFilePair,
};
use super::strategy_codec::{build_hand_strategy, DecodedHandStrategy};

pub const DEFAULT_STRATEGY_CACHE_BYTE_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct HandStrategyStoreOptions {
    pub cache_byte_budget: usize,
}

impl Default for HandStrategyStoreOptions {
    fn default() -> Self {
        Self {
            cache_byte_budget: DEFAULT_STRATEGY_CACHE_BYTE_BUDGET,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HandStrategyExportOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub overwrite: bool,
}

pub fn export_hand_strategies(
    options: &HandStrategyExportOptions,
    concrete_paths: &[ExportedConcreteActionPath],
) -> Result<HandStrategiesManifest, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    validate_concrete_path_ids(concrete_paths)?;
    fs::create_dir_all(&options.out_dir)?;
    let files = StagedFilePair::new(
        &options.out_dir,
        HAND_STRATEGIES_DATA_FILE_NAME,
        HAND_STRATEGIES_INDEX_FILE_NAME,
    );
    if !options.overwrite && files.final_paths().into_iter().any(Path::exists) {
        return Err(ToolError::invalid_argument(
            "V3 hand strategy output already exists",
        ));
    }
    files.remove_temporary_files();

    let connection = Connection::open(&options.source_db, true)?;
    let result = write_strategy_files(
        &connection,
        &options.dimension,
        concrete_paths,
        &files.data.temporary_path,
        &files.index.temporary_path,
    );
    let manifest = match result {
        Ok(manifest) => manifest,
        Err(error) => {
            files.remove_temporary_files();
            return Err(error);
        }
    };
    files.publish(options.overwrite)?;
    Ok(manifest)
}

pub struct HandStrategyStore {
    data_mmap: Mmap,
    index_mmap: Mmap,
    _data_file: File,
    _index_file: File,
    record_count: u64,
    locator_offset: usize,
    cache: Mutex<ByteLru<u32, DecodedHandStrategy>>,
}

impl HandStrategyStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(dir, HandStrategyStoreOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: HandStrategyStoreOptions,
    ) -> Result<Self, ToolError> {
        let dir = dir.as_ref();
        let data_file = File::open(dir.join(HAND_STRATEGIES_DATA_FILE_NAME))?;
        let index_file = File::open(dir.join(HAND_STRATEGIES_INDEX_FILE_NAME))?;
        // SAFETY: V3 files are opened read-only, retained for the mapping lifetime, and must not be
        // mutated while this reader is alive.
        let data_mmap = unsafe { Mmap::map(&data_file)? };
        // SAFETY: same immutable-file contract as the data mapping above.
        let index_mmap = unsafe { Mmap::map(&index_file)? };
        let data_header = decode_header(&data_mmap, FileKind::HandStrategiesData)?;
        let index_header = decode_header(&index_mmap, FileKind::HandStrategiesIndex)?;
        if data_header.primary_count == 0
            || data_header.primary_count != index_header.primary_count
            || data_header.secondary_count != 0
            || index_header.secondary_count != 0
        {
            return Err(invalid_store(
                "V3 hand strategy data/index record counts are invalid",
            ));
        }
        if index_header.section_count != 1 {
            return Err(invalid_store(
                "V3 hand strategy index must contain one locator section",
            ));
        }
        let descriptor_end = HEADER_SIZE + SECTION_DESCRIPTOR_SIZE;
        let descriptor =
            decode_section_descriptor(index_mmap.get(HEADER_SIZE..descriptor_end).ok_or_else(
                || invalid_store("V3 hand strategy section directory is truncated"),
            )?)?;
        if descriptor.kind != SectionKind::PayloadLocators
            || usize::from(descriptor.record_size) != PAYLOAD_LOCATOR_SIZE
            || descriptor.record_count != index_header.primary_count
            || descriptor.offset as usize != descriptor_end
            || descriptor.end()? != index_mmap.len() as u64
        {
            return Err(invalid_store("V3 hand strategy locator section is invalid"));
        }
        Ok(Self {
            data_mmap,
            index_mmap,
            _data_file: data_file,
            _index_file: index_file,
            record_count: data_header.primary_count,
            locator_offset: descriptor_end,
            cache: Mutex::new(ByteLru::new(options.cache_byte_budget)),
        })
    }

    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    pub fn cache_stats(&self) -> ByteCacheStats {
        self.cache
            .lock()
            .expect("V3 strategy cache lock poisoned")
            .stats()
    }

    pub(crate) fn resize_cache_budget(&self, byte_budget: usize) {
        self.cache
            .lock()
            .expect("V3 strategy cache lock poisoned")
            .resize(byte_budget);
    }

    pub(crate) fn cache_byte_budget(&self) -> usize {
        self.cache
            .lock()
            .expect("V3 strategy cache lock poisoned")
            .byte_budget()
    }

    /// Decode every strategy payload and verify that locators densely and contiguously cover the
    /// data file. Decoding also enforces all schema, bitmap, array and null-sentinel invariants.
    pub fn verify_and_snapshot(&self) -> Result<Vec<HandStrategy>, ToolError> {
        let count = u32::try_from(self.record_count)
            .map_err(|_| invalid_store("V3 hand strategy count exceeds uint32"))?;
        let mut expected_offset = HEADER_SIZE as u64;
        let mut strategies = Vec::with_capacity(count as usize);
        for concrete_action_path_id in 1..=count {
            let locator = self.locator(concrete_action_path_id)?;
            if locator.byte_length == 0 || locator.offset != expected_offset {
                return Err(invalid_store(format!(
                    "V3 strategy locator for id {concrete_action_path_id} is empty or non-contiguous"
                )));
            }
            expected_offset = locator
                .offset
                .checked_add(u64::from(locator.byte_length))
                .ok_or_else(|| invalid_store("V3 strategy payload end overflow"))?;
            strategies.push(
                self.read_uncached(concrete_action_path_id)?
                    .strategy()
                    .clone(),
            );
        }
        if expected_offset != self.data_mmap.len() as u64 {
            return Err(invalid_store(
                "V3 strategy payloads do not exactly cover the data file",
            ));
        }
        Ok(strategies)
    }

    pub fn read(
        &self,
        concrete_action_path_id: u32,
    ) -> Result<Arc<DecodedHandStrategy>, ToolError> {
        if concrete_action_path_id == 0 || u64::from(concrete_action_path_id) > self.record_count {
            return Err(ToolError::new(
                "CONCRETE_LINE_NOT_FOUND",
                format!("V3 concrete action path id {concrete_action_path_id} is out of range"),
            ));
        }
        {
            let mut cache = self.cache.lock().expect("V3 strategy cache lock poisoned");
            if let Some(strategy) = cache.get(concrete_action_path_id) {
                return Ok(strategy);
            }
        }
        let strategy = Arc::new(self.read_uncached(concrete_action_path_id)?);
        let estimated_bytes = strategy.estimated_heap_bytes();
        self.cache
            .lock()
            .expect("V3 strategy cache lock poisoned")
            .put(
                concrete_action_path_id,
                Arc::clone(&strategy),
                estimated_bytes,
            );
        Ok(strategy)
    }

    fn read_uncached(
        &self,
        concrete_action_path_id: u32,
    ) -> Result<DecodedHandStrategy, ToolError> {
        let locator = self.locator(concrete_action_path_id)?;
        let payload_end = locator
            .offset
            .checked_add(u64::from(locator.byte_length))
            .ok_or_else(|| invalid_store("V3 strategy payload end overflow"))?;
        if locator.offset < HEADER_SIZE as u64 || payload_end > self.data_mmap.len() as u64 {
            return Err(invalid_store(
                "V3 strategy locator points outside data file",
            ));
        }
        let start = usize::try_from(locator.offset)
            .map_err(|_| invalid_store("V3 strategy payload offset exceeds usize"))?;
        let end = usize::try_from(payload_end)
            .map_err(|_| invalid_store("V3 strategy payload end exceeds usize"))?;
        let payload = &self.data_mmap[start..end];
        assert_crc32c(payload, locator.crc32c).map_err(invalid_store)?;
        let strategy = HandStrategy::decode(payload)
            .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))?;
        DecodedHandStrategy::new(strategy)
    }

    fn locator(&self, concrete_action_path_id: u32) -> Result<PayloadLocator, ToolError> {
        if concrete_action_path_id == 0 || u64::from(concrete_action_path_id) > self.record_count {
            return Err(invalid_store(format!(
                "V3 concrete action path id {concrete_action_path_id} is out of bounds"
            )));
        }
        let record_index = concrete_action_path_id as usize - 1;
        let start = self
            .locator_offset
            .checked_add(
                record_index
                    .checked_mul(PAYLOAD_LOCATOR_SIZE)
                    .ok_or_else(|| invalid_store("V3 strategy locator offset overflow"))?,
            )
            .ok_or_else(|| invalid_store("V3 strategy locator offset overflow"))?;
        decode_payload_locator(
            self.index_mmap
                .get(start..start + PAYLOAD_LOCATOR_SIZE)
                .ok_or_else(|| invalid_store("V3 strategy locator is truncated"))?,
        )
    }
}

fn write_strategy_files(
    connection: &Connection,
    dimension: &DimensionSpec,
    concrete_paths: &[ExportedConcreteActionPath],
    data_path: &Path,
    index_path: &Path,
) -> Result<HandStrategiesManifest, ToolError> {
    let record_count = concrete_paths.len() as u64;
    let locators = write_payload_data_file(
        data_path,
        FileKind::HandStrategiesData,
        concrete_paths.len(),
        0,
        |path_index| {
            let rows =
                load_strategy_rows(connection, dimension, concrete_paths[path_index].source_id)?;
            Ok(build_hand_strategy(&rows)?.encode_to_vec())
        },
        || {
            ToolError::new(
                "V3_HAND_STRATEGY_TOO_LARGE",
                "Encoded V3 hand strategy exceeds uint32",
            )
        },
        || invalid_store("V3 hand strategy data offset overflow"),
    )?;
    write_index_file(
        index_path,
        FileKind::HandStrategiesIndex,
        record_count,
        0,
        vec![payload_locator_section(
            SectionKind::PayloadLocators,
            &locators,
        )],
    )?;

    Ok(HandStrategiesManifest {
        data: manifest_file(data_path, HAND_STRATEGIES_DATA_FILE_NAME, record_count, 0)?,
        index: manifest_file(index_path, HAND_STRATEGIES_INDEX_FILE_NAME, record_count, 0)?,
        record_count,
    })
}

fn validate_concrete_path_ids(
    concrete_paths: &[ExportedConcreteActionPath],
) -> Result<(), ToolError> {
    if concrete_paths.is_empty() {
        return Err(ToolError::new(
            "V3_HAND_STRATEGIES_EMPTY",
            "Cannot export strategies without concrete action paths",
        ));
    }
    for (index, path) in concrete_paths.iter().enumerate() {
        let expected = u32::try_from(index + 1)
            .map_err(|_| invalid_store("V3 concrete action path count exceeds uint32"))?;
        if path.concrete_action_path_id != expected {
            return Err(ToolError::new(
                "NON_DENSE_V3_CONCRETE_ACTION_PATH_IDS",
                format!(
                    "Expected V3 concrete action path id {expected}, got {}",
                    path.concrete_action_path_id
                ),
            ));
        }
    }
    Ok(())
}

fn invalid_store(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_V3_HAND_STRATEGY_STORE", message)
}
