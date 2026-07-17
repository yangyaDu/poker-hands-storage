use std::collections::HashSet;
use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};

use memmap2::Mmap;
use prost::Message;
use range_store_core::crc32c::assert_crc32c;
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};

use crate::errors::ToolError;

use super::cache::{ByteCacheStats, ByteLru};
use super::format::{
    checked_u32_index, decode_hash_locator, decode_header, decode_payload_locator,
    decode_section_descriptor, sort_hash_locators, stable_hash64, FileKind, HashLocator,
    PayloadLocator, SectionKind, ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
    ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME, DRILL_SCENARIOS_DATA_FILE_NAME,
    DRILL_SCENARIOS_INDEX_FILE_NAME, HASH_LOCATOR_SIZE, HEADER_SIZE, NO_VALUE_INDEX,
    PAYLOAD_LOCATOR_SIZE, SECTION_DESCRIPTOR_SIZE,
};
pub use super::metadata_export::{
    export_metadata, ExportedConcreteActionPath, MetadataExportOptions, MetadataExportSummary,
    DEFAULT_METADATA_PAGE_TARGET_BYTES,
};
use super::proto::{
    AbstractActionPathEntry, AbstractActionPathPage, DrillScenarioEntry, DrillScenarioPage,
};
pub const DEFAULT_METADATA_CACHE_BYTE_BUDGET: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct MetadataStoreOptions {
    pub page_cache_byte_budget: usize,
}

impl Default for MetadataStoreOptions {
    fn default() -> Self {
        Self {
            page_cache_byte_budget: DEFAULT_METADATA_CACHE_BYTE_BUDGET,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MetadataSnapshot {
    pub drill_scenarios: Vec<DrillScenarioEntry>,
    pub abstract_action_paths: Vec<AbstractActionPathEntry>,
    pub concrete_path_count: u64,
}

pub struct MetadataStore {
    drill: PagedDataset,
    action_paths: PagedDataset,
    drill_cache: Mutex<ByteLru<u32, DrillScenarioPage>>,
    action_path_cache: Mutex<ByteLru<u32, AbstractActionPathPage>>,
}

impl MetadataStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        Self::open_with_options(dir, MetadataStoreOptions::default())
    }

    pub fn open_with_options(
        dir: impl AsRef<Path>,
        options: MetadataStoreOptions,
    ) -> Result<Self, ToolError> {
        let dir = dir.as_ref();
        let drill_budget = options.page_cache_byte_budget / 2;
        let action_path_budget = options.page_cache_byte_budget - drill_budget;
        Ok(Self {
            drill: PagedDataset::open(
                &dir.join(DRILL_SCENARIOS_DATA_FILE_NAME),
                &dir.join(DRILL_SCENARIOS_INDEX_FILE_NAME),
                FileKind::DrillScenariosData,
                FileKind::DrillScenariosIndex,
                &[SectionKind::PageLocators, SectionKind::PrimaryHashLocators],
            )?,
            action_paths: PagedDataset::open(
                &dir.join(ABSTRACT_ACTION_PATHS_DATA_FILE_NAME),
                &dir.join(ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME),
                FileKind::AbstractActionPathsData,
                FileKind::AbstractActionPathsIndex,
                &[
                    SectionKind::PageLocators,
                    SectionKind::PrimaryHashLocators,
                    SectionKind::SecondaryHashLocators,
                ],
            )?,
            drill_cache: Mutex::new(ByteLru::new(drill_budget)),
            action_path_cache: Mutex::new(ByteLru::new(action_path_budget)),
        })
    }

    pub fn cache_stats(&self) -> ByteCacheStats {
        let mut stats = self
            .drill_cache
            .lock()
            .expect("V3 drill page cache lock poisoned")
            .stats();
        stats.merge(
            self.action_path_cache
                .lock()
                .expect("V3 action path page cache lock poisoned")
                .stats(),
        );
        stats
    }

    pub(crate) fn resize_cache_budget(&self, page_cache_byte_budget: usize) {
        let drill_budget = page_cache_byte_budget / 2;
        let action_path_budget = page_cache_byte_budget - drill_budget;
        self.drill_cache
            .lock()
            .expect("V3 drill page cache lock poisoned")
            .resize(drill_budget);
        self.action_path_cache
            .lock()
            .expect("V3 action path page cache lock poisoned")
            .resize(action_path_budget);
    }

    pub(crate) fn cache_byte_budget(&self) -> usize {
        self.drill_cache
            .lock()
            .expect("V3 drill page cache lock poisoned")
            .byte_budget()
            + self
                .action_path_cache
                .lock()
                .expect("V3 action path page cache lock poisoned")
                .byte_budget()
    }

    /// Decode every metadata page and prove that the fixed-width indexes are an exact,
    /// one-to-one projection of the protobuf payloads.
    pub fn verify_and_snapshot(&self) -> Result<MetadataSnapshot, ToolError> {
        self.drill.verify_payload_layout()?;
        self.action_paths.verify_payload_layout()?;

        let mut drill_scenarios = Vec::new();
        let mut expected_drill_locators = Vec::new();
        for page_id in 0..self.drill.page_count_u32()? {
            let page: DrillScenarioPage = self.drill.read_page(page_id)?;
            if page.entries.is_empty() {
                return Err(invalid_metadata(format!(
                    "Drill page {page_id} must not be empty"
                )));
            }
            for (entry_index, entry) in page.entries.into_iter().enumerate() {
                expected_drill_locators.push(HashLocator::entry(
                    stable_hash64(&entry.drill_name),
                    page_id,
                    checked_u32_index("drill entry index", entry_index)?,
                ));
                drill_scenarios.push(entry);
            }
        }
        if drill_scenarios.len() as u64 != self.drill.data_secondary_count {
            return Err(invalid_metadata(
                "Drill entry count does not match the data header",
            ));
        }
        sort_hash_locators(&mut expected_drill_locators);
        self.drill
            .verify_hash_section(SectionKind::PrimaryHashLocators, &expected_drill_locators)?;

        let mut abstract_action_paths = Vec::new();
        let mut expected_abstract_locators = Vec::new();
        let mut expected_concrete_locators = Vec::new();
        for page_id in 0..self.action_paths.page_count_u32()? {
            let page: AbstractActionPathPage = self.action_paths.read_page(page_id)?;
            if page.entries.is_empty() {
                return Err(invalid_metadata(format!(
                    "Abstract action path page {page_id} must not be empty"
                )));
            }
            for (entry_index, entry) in page.entries.into_iter().enumerate() {
                let entry_index =
                    checked_u32_index("abstract action path entry index", entry_index)?;
                expected_abstract_locators.push(HashLocator::entry(
                    stable_hash64(&entry.abstract_action_path),
                    page_id,
                    entry_index,
                ));
                for (value_index, path) in entry.concrete_action_paths.iter().enumerate() {
                    expected_concrete_locators.push(HashLocator::value(
                        stable_hash64(&path.concrete_action_path),
                        page_id,
                        entry_index,
                        checked_u32_index("concrete action path value index", value_index)?,
                    ));
                }
                abstract_action_paths.push(entry);
            }
        }
        if abstract_action_paths.len() as u64 != self.action_paths.data_secondary_count {
            return Err(invalid_metadata(
                "Abstract action path count does not match the data header",
            ));
        }
        sort_hash_locators(&mut expected_abstract_locators);
        sort_hash_locators(&mut expected_concrete_locators);
        self.action_paths.verify_hash_section(
            SectionKind::PrimaryHashLocators,
            &expected_abstract_locators,
        )?;
        self.action_paths.verify_hash_section(
            SectionKind::SecondaryHashLocators,
            &expected_concrete_locators,
        )?;

        let mut drill_names = HashSet::new();
        let mut abstract_paths = HashSet::new();
        let mut concrete_paths = HashSet::new();
        let mut concrete_ids = Vec::new();
        for drill in &drill_scenarios {
            if !drill_names.insert(drill.drill_name.as_str()) {
                return Err(invalid_metadata(format!(
                    "Duplicate drill scenario {:?}",
                    drill.drill_name
                )));
            }
        }
        for entry in &abstract_action_paths {
            if !abstract_paths.insert(entry.abstract_action_path.as_str()) {
                return Err(invalid_metadata(format!(
                    "Duplicate abstract action path {:?}",
                    entry.abstract_action_path
                )));
            }
            for concrete in &entry.concrete_action_paths {
                if !concrete_paths.insert(concrete.concrete_action_path.as_str()) {
                    return Err(invalid_metadata(format!(
                        "Duplicate concrete action path {:?}",
                        concrete.concrete_action_path
                    )));
                }
                concrete_ids.push(concrete.concrete_action_path_id);
            }
        }
        for drill in &drill_scenarios {
            for abstract_path in &drill.abstract_action_paths {
                if !abstract_paths.contains(abstract_path.as_str()) {
                    return Err(invalid_metadata(format!(
                        "Drill {:?} references missing abstract action path {:?}",
                        drill.drill_name, abstract_path
                    )));
                }
            }
        }
        concrete_ids.sort_unstable();
        for (index, actual) in concrete_ids.iter().copied().enumerate() {
            let expected = checked_u32_index("concrete action path id", index + 1)?;
            if actual != expected {
                return Err(invalid_metadata(format!(
                    "Concrete action path ids must be dense: expected {expected}, got {actual}"
                )));
            }
        }

        Ok(MetadataSnapshot {
            drill_scenarios,
            abstract_action_paths,
            concrete_path_count: concrete_ids.len() as u64,
        })
    }

    pub fn get_drill_scenario_lines(&self, drill_name: &str) -> Result<Vec<String>, ToolError> {
        let hash = stable_hash64(drill_name);
        let mut matched = None;
        for locator in self
            .drill
            .hash_locators(SectionKind::PrimaryHashLocators, hash)?
        {
            require_entry_locator(locator, "drill")?;
            let page = self.read_drill_page(locator.page_id)?;
            let entry = page
                .entries
                .get(locator.entry_index as usize)
                .ok_or_else(|| {
                    invalid_metadata("Drill hash locator entry index is out of bounds")
                })?;
            if entry.drill_name != drill_name {
                continue;
            }
            if matched
                .replace(entry.abstract_action_paths.clone())
                .is_some()
            {
                return Err(invalid_metadata(format!(
                    "Duplicate drill scenario entry for {drill_name:?}"
                )));
            }
        }
        matched.ok_or_else(|| {
            ToolError::new(
                "DRILL_SCENARIO_NOT_FOUND",
                format!("No abstract action paths found for drill {drill_name:?}"),
            )
        })
    }

    pub fn get_concrete_lines(
        &self,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, ToolError> {
        match filter {
            ConcreteLineFilter::Abstract(abstract_path) => {
                let entry = self.find_abstract_action_path(abstract_path)?;
                Ok(entry
                    .concrete_action_paths
                    .into_iter()
                    .map(|path| ConcreteLineRow {
                        concrete_line_id: path.concrete_action_path_id,
                        abstract_line: entry.abstract_action_path.clone(),
                        concrete_line: path.concrete_action_path,
                    })
                    .collect())
            }
            ConcreteLineFilter::Concrete(concrete_path) => {
                Ok(vec![self.find_concrete_action_path(concrete_path)?])
            }
            ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            } => {
                let row = self.find_concrete_action_path(concrete_line)?;
                if row.abstract_line != abstract_line {
                    return Err(concrete_not_found());
                }
                Ok(vec![row])
            }
        }
    }

    pub fn resolve_concrete_action_path(
        &self,
        concrete_action_path: &str,
    ) -> Result<u32, ToolError> {
        Ok(self
            .find_concrete_action_path(concrete_action_path)?
            .concrete_line_id)
    }

    fn find_abstract_action_path(
        &self,
        abstract_path: &str,
    ) -> Result<AbstractActionPathEntry, ToolError> {
        let hash = stable_hash64(abstract_path);
        let mut matched = None;
        for locator in self
            .action_paths
            .hash_locators(SectionKind::PrimaryHashLocators, hash)?
        {
            require_entry_locator(locator, "abstract action path")?;
            let page = self.read_action_path_page(locator.page_id)?;
            let entry = page
                .entries
                .get(locator.entry_index as usize)
                .ok_or_else(|| {
                    invalid_metadata("Abstract action path locator entry index is out of bounds")
                })?;
            if entry.abstract_action_path != abstract_path {
                continue;
            }
            if matched.replace(entry.clone()).is_some() {
                return Err(invalid_metadata(format!(
                    "Duplicate abstract action path entry for {abstract_path:?}"
                )));
            }
        }
        matched.ok_or_else(concrete_not_found)
    }

    fn find_concrete_action_path(&self, concrete_path: &str) -> Result<ConcreteLineRow, ToolError> {
        let hash = stable_hash64(concrete_path);
        let mut matched = None;
        for locator in self
            .action_paths
            .hash_locators(SectionKind::SecondaryHashLocators, hash)?
        {
            if locator.value_index == NO_VALUE_INDEX {
                return Err(invalid_metadata(
                    "Concrete action path locator is missing value index",
                ));
            }
            let page = self.read_action_path_page(locator.page_id)?;
            let entry = page
                .entries
                .get(locator.entry_index as usize)
                .ok_or_else(|| {
                    invalid_metadata("Concrete action path locator entry index is out of bounds")
                })?;
            let path = entry
                .concrete_action_paths
                .get(locator.value_index as usize)
                .ok_or_else(|| {
                    invalid_metadata("Concrete action path locator value index is out of bounds")
                })?;
            if path.concrete_action_path != concrete_path {
                continue;
            }
            let row = ConcreteLineRow {
                concrete_line_id: path.concrete_action_path_id,
                abstract_line: entry.abstract_action_path.clone(),
                concrete_line: path.concrete_action_path.clone(),
            };
            if matched.replace(row).is_some() {
                return Err(invalid_metadata(format!(
                    "Duplicate concrete action path entry for {concrete_path:?}"
                )));
            }
        }
        matched.ok_or_else(concrete_not_found)
    }

    fn read_drill_page(&self, page_id: u32) -> Result<Arc<DrillScenarioPage>, ToolError> {
        {
            let mut cache = self
                .drill_cache
                .lock()
                .expect("V3 drill page cache lock poisoned");
            if let Some(page) = cache.get(page_id) {
                return Ok(page);
            }
        }
        let page = Arc::new(self.drill.read_page(page_id)?);
        let estimated_bytes = estimate_drill_page_bytes(&page);
        self.drill_cache
            .lock()
            .expect("V3 drill page cache lock poisoned")
            .put(page_id, Arc::clone(&page), estimated_bytes);
        Ok(page)
    }

    fn read_action_path_page(
        &self,
        page_id: u32,
    ) -> Result<Arc<AbstractActionPathPage>, ToolError> {
        {
            let mut cache = self
                .action_path_cache
                .lock()
                .expect("V3 action path page cache lock poisoned");
            if let Some(page) = cache.get(page_id) {
                return Ok(page);
            }
        }
        let page = Arc::new(self.action_paths.read_page(page_id)?);
        let estimated_bytes = estimate_action_path_page_bytes(&page);
        self.action_path_cache
            .lock()
            .expect("V3 action path page cache lock poisoned")
            .put(page_id, Arc::clone(&page), estimated_bytes);
        Ok(page)
    }
}

fn estimate_drill_page_bytes(page: &DrillScenarioPage) -> usize {
    std::mem::size_of::<DrillScenarioPage>()
        + page.entries.capacity() * std::mem::size_of::<super::proto::DrillScenarioEntry>()
        + page
            .entries
            .iter()
            .map(|entry| {
                entry.drill_name.capacity()
                    + entry.abstract_action_paths.capacity() * std::mem::size_of::<String>()
                    + entry
                        .abstract_action_paths
                        .iter()
                        .map(String::capacity)
                        .sum::<usize>()
            })
            .sum::<usize>()
}

fn estimate_action_path_page_bytes(page: &AbstractActionPathPage) -> usize {
    std::mem::size_of::<AbstractActionPathPage>()
        + page.entries.capacity() * std::mem::size_of::<AbstractActionPathEntry>()
        + page
            .entries
            .iter()
            .map(|entry| {
                entry.abstract_action_path.capacity()
                    + entry.concrete_action_paths.capacity()
                        * std::mem::size_of::<super::proto::ConcreteActionPathRef>()
                    + entry
                        .concrete_action_paths
                        .iter()
                        .map(|path| path.concrete_action_path.capacity())
                        .sum::<usize>()
            })
            .sum::<usize>()
}

struct PagedDataset {
    data_mmap: Mmap,
    index_mmap: Mmap,
    _data_file: File,
    _index_file: File,
    page_locators: SectionView,
    primary_hash_locators: SectionView,
    secondary_hash_locators: Option<SectionView>,
    data_secondary_count: u64,
}

impl PagedDataset {
    fn open(
        data_path: &Path,
        index_path: &Path,
        data_kind: FileKind,
        index_kind: FileKind,
        expected_sections: &[SectionKind],
    ) -> Result<Self, ToolError> {
        let data_file = File::open(data_path)?;
        let index_file = File::open(index_path)?;
        // SAFETY: V3 files are opened read-only, retained for the mapping lifetime, and must not be
        // mutated while this reader is alive.
        let data_mmap = unsafe { Mmap::map(&data_file)? };
        // SAFETY: same immutable-file contract as the data mapping above.
        let index_mmap = unsafe { Mmap::map(&index_file)? };
        let data_header = decode_header(&data_mmap, data_kind)?;
        let index_header = decode_header(&index_mmap, index_kind)?;
        if data_header.primary_count != index_header.primary_count {
            return Err(invalid_metadata(
                "V3 metadata data/index page counts do not match",
            ));
        }
        if index_header.section_count as usize != expected_sections.len() {
            return Err(invalid_metadata(format!(
                "V3 metadata index contains {} sections, expected {}",
                index_header.section_count,
                expected_sections.len()
            )));
        }
        let sections = parse_sections(&index_mmap, expected_sections)?;
        if sections[0].record_count != index_header.primary_count {
            return Err(invalid_metadata(
                "V3 metadata page locator count does not match header",
            ));
        }
        let hash_record_count = sections[1..].iter().try_fold(0u64, |total, section| {
            total
                .checked_add(section.record_count)
                .ok_or_else(|| invalid_metadata("V3 metadata hash count overflow"))
        })?;
        if hash_record_count != index_header.secondary_count {
            return Err(invalid_metadata(
                "V3 metadata hash locator count does not match header",
            ));
        }
        Ok(Self {
            data_mmap,
            index_mmap,
            _data_file: data_file,
            _index_file: index_file,
            page_locators: sections[0],
            primary_hash_locators: sections[1],
            secondary_hash_locators: sections.get(2).copied(),
            data_secondary_count: data_header.secondary_count,
        })
    }

    fn page_count_u32(&self) -> Result<u32, ToolError> {
        u32::try_from(self.page_locators.record_count)
            .map_err(|_| invalid_metadata("V3 metadata page count exceeds uint32"))
    }

    fn verify_payload_layout(&self) -> Result<(), ToolError> {
        let mut expected_offset = HEADER_SIZE as u64;
        for page_id in 0..self.page_count_u32()? {
            let locator = self.payload_locator(page_id)?;
            if locator.byte_length == 0 || locator.offset != expected_offset {
                return Err(invalid_metadata(format!(
                    "V3 metadata page {page_id} locator is empty or non-contiguous"
                )));
            }
            expected_offset = locator
                .offset
                .checked_add(u64::from(locator.byte_length))
                .ok_or_else(|| invalid_metadata("V3 metadata page end overflow"))?;
        }
        if expected_offset != self.data_mmap.len() as u64 {
            return Err(invalid_metadata(
                "V3 metadata payloads do not exactly cover the data file",
            ));
        }
        Ok(())
    }

    fn verify_hash_section(
        &self,
        kind: SectionKind,
        expected: &[HashLocator],
    ) -> Result<(), ToolError> {
        let section = match kind {
            SectionKind::PrimaryHashLocators => self.primary_hash_locators,
            SectionKind::SecondaryHashLocators => self
                .secondary_hash_locators
                .ok_or_else(|| invalid_metadata("V3 metadata secondary hash index is missing"))?,
            _ => return Err(invalid_metadata("V3 metadata hash section kind is invalid")),
        };
        if section.record_count != expected.len() as u64 {
            return Err(invalid_metadata(format!(
                "V3 metadata {:?} count does not match payloads",
                kind
            )));
        }
        for (index, expected_locator) in expected.iter().enumerate() {
            let actual = decode_hash_locator(section.record(&self.index_mmap, index)?)?;
            if actual.reserved != 0 || actual != *expected_locator {
                return Err(invalid_metadata(format!(
                    "V3 metadata {:?} locator {index} does not match payloads",
                    kind
                )));
            }
        }
        Ok(())
    }

    fn read_page<P: Message + Default>(&self, page_id: u32) -> Result<P, ToolError> {
        let locator = self.payload_locator(page_id)?;
        let end = locator
            .offset
            .checked_add(u64::from(locator.byte_length))
            .ok_or_else(|| invalid_metadata("V3 metadata page end overflow"))?;
        if locator.offset < HEADER_SIZE as u64 || end > self.data_mmap.len() as u64 {
            return Err(invalid_metadata(
                "V3 metadata page locator points outside data file",
            ));
        }
        let start = usize::try_from(locator.offset)
            .map_err(|_| invalid_metadata("V3 metadata page offset exceeds usize"))?;
        let end = usize::try_from(end)
            .map_err(|_| invalid_metadata("V3 metadata page end exceeds usize"))?;
        let payload = &self.data_mmap[start..end];
        assert_crc32c(payload, locator.crc32c).map_err(invalid_metadata)?;
        P::decode(payload)
            .map_err(|error| ToolError::new("PROTOBUF_DECODE_ERROR", error.to_string()))
    }

    fn payload_locator(&self, page_id: u32) -> Result<PayloadLocator, ToolError> {
        let index = page_id as usize;
        if index >= self.page_locators.record_count as usize {
            return Err(invalid_metadata(format!(
                "V3 metadata page id {page_id} is out of bounds"
            )));
        }
        decode_payload_locator(self.page_locators.record(&self.index_mmap, index)?)
    }

    fn hash_locators(&self, kind: SectionKind, hash: u64) -> Result<Vec<HashLocator>, ToolError> {
        let section = match kind {
            SectionKind::PrimaryHashLocators => self.primary_hash_locators,
            SectionKind::SecondaryHashLocators => self
                .secondary_hash_locators
                .ok_or_else(|| invalid_metadata("V3 metadata secondary hash index is missing"))?,
            _ => {
                return Err(invalid_metadata(
                    "Requested V3 metadata section is not a hash index",
                ));
            }
        };
        let range = section.equal_hash_range(&self.index_mmap, hash)?;
        range
            .map(|index| decode_hash_locator(section.record(&self.index_mmap, index)?))
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct SectionView {
    offset: usize,
    record_count: u64,
    record_size: usize,
}

impl SectionView {
    fn record<'a>(&self, index_bytes: &'a [u8], index: usize) -> Result<&'a [u8], ToolError> {
        if index >= self.record_count as usize {
            return Err(invalid_metadata("V3 index record is out of bounds"));
        }
        let start = self
            .offset
            .checked_add(
                index
                    .checked_mul(self.record_size)
                    .ok_or_else(|| invalid_metadata("V3 index record offset overflow"))?,
            )
            .ok_or_else(|| invalid_metadata("V3 index record offset overflow"))?;
        let end = start
            .checked_add(self.record_size)
            .ok_or_else(|| invalid_metadata("V3 index record end overflow"))?;
        index_bytes
            .get(start..end)
            .ok_or_else(|| invalid_metadata("V3 index record is truncated"))
    }

    fn hash_at(&self, index_bytes: &[u8], index: usize) -> Result<u64, ToolError> {
        Ok(decode_hash_locator(self.record(index_bytes, index)?)?.hash)
    }

    fn equal_hash_range(
        &self,
        index_bytes: &[u8],
        hash: u64,
    ) -> Result<std::ops::Range<usize>, ToolError> {
        let count = usize::try_from(self.record_count)
            .map_err(|_| invalid_metadata("V3 hash record count exceeds usize"))?;
        let lower = self.partition_point(index_bytes, count, |candidate| candidate < hash)?;
        let upper = self.partition_point(index_bytes, count, |candidate| candidate <= hash)?;
        Ok(lower..upper)
    }

    fn partition_point(
        &self,
        index_bytes: &[u8],
        count: usize,
        predicate: impl Fn(u64) -> bool,
    ) -> Result<usize, ToolError> {
        let mut left = 0;
        let mut right = count;
        while left < right {
            let middle = left + (right - left) / 2;
            if predicate(self.hash_at(index_bytes, middle)?) {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        Ok(left)
    }
}

fn parse_sections(
    index_bytes: &[u8],
    expected_kinds: &[SectionKind],
) -> Result<Vec<SectionView>, ToolError> {
    let directory_end = HEADER_SIZE
        .checked_add(
            expected_kinds
                .len()
                .checked_mul(SECTION_DESCRIPTOR_SIZE)
                .ok_or_else(|| invalid_metadata("V3 section directory size overflow"))?,
        )
        .ok_or_else(|| invalid_metadata("V3 section directory end overflow"))?;
    if directory_end > index_bytes.len() {
        return Err(invalid_metadata("V3 section directory is truncated"));
    }
    let mut expected_offset = directory_end;
    let mut sections = Vec::with_capacity(expected_kinds.len());
    for (index, expected_kind) in expected_kinds.iter().enumerate() {
        let descriptor_start = HEADER_SIZE + index * SECTION_DESCRIPTOR_SIZE;
        let descriptor = decode_section_descriptor(
            &index_bytes[descriptor_start..descriptor_start + SECTION_DESCRIPTOR_SIZE],
        )?;
        if descriptor.kind != *expected_kind {
            return Err(invalid_metadata(format!(
                "Unexpected V3 metadata section {:?}, expected {:?}",
                descriptor.kind, expected_kind
            )));
        }
        let expected_record_size = match descriptor.kind {
            SectionKind::PageLocators => PAYLOAD_LOCATOR_SIZE,
            SectionKind::PrimaryHashLocators | SectionKind::SecondaryHashLocators => {
                HASH_LOCATOR_SIZE
            }
            SectionKind::PayloadLocators => {
                return Err(invalid_metadata(
                    "Hand strategy payload locator section is invalid in metadata index",
                ));
            }
        };
        if usize::from(descriptor.record_size) != expected_record_size {
            return Err(invalid_metadata(format!(
                "V3 metadata section {:?} has invalid record size {}",
                descriptor.kind, descriptor.record_size
            )));
        }
        let offset = usize::try_from(descriptor.offset)
            .map_err(|_| invalid_metadata("V3 section offset exceeds usize"))?;
        let end = usize::try_from(descriptor.end()?)
            .map_err(|_| invalid_metadata("V3 section end exceeds usize"))?;
        if offset != expected_offset || end > index_bytes.len() {
            return Err(invalid_metadata(
                "V3 metadata sections must be contiguous and inside the index file",
            ));
        }
        expected_offset = end;
        sections.push(SectionView {
            offset,
            record_count: descriptor.record_count,
            record_size: expected_record_size,
        });
    }
    if expected_offset != index_bytes.len() {
        return Err(invalid_metadata(
            "V3 metadata index contains unexpected trailing bytes",
        ));
    }
    Ok(sections)
}

fn require_entry_locator(locator: HashLocator, name: &str) -> Result<(), ToolError> {
    if locator.value_index != NO_VALUE_INDEX {
        return Err(invalid_metadata(format!(
            "V3 {name} locator must not contain a value index"
        )));
    }
    Ok(())
}

fn invalid_metadata(message: impl Into<String>) -> ToolError {
    ToolError::new("INVALID_V3_METADATA", message)
}

fn concrete_not_found() -> ToolError {
    ToolError::new("CONCRETE_LINE_NOT_FOUND", "No concrete action paths match")
}
