use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use prost::Message;
use range_store_core::crc32c::{assert_crc32c, crc32c};
use range_store_core::dimension::DimensionSpec;
use range_store_core::metadata::{ConcreteLineFilter, ConcreteLineRow};
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

use super::format::{
    decode_hash_locator, decode_header, decode_payload_locator, decode_section_descriptor,
    encode_hash_locator, encode_header, encode_payload_locator, encode_section_descriptor,
    stable_hash64, FileHeader, FileKind, HashLocator, PayloadLocator, SectionDescriptor,
    SectionKind, ABSTRACT_ACTION_PATHS_DATA_FILE_NAME, ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
    DRILL_SCENARIOS_DATA_FILE_NAME, DRILL_SCENARIOS_INDEX_FILE_NAME, HASH_LOCATOR_SIZE,
    HEADER_SIZE, NO_VALUE_INDEX, PAYLOAD_LOCATOR_SIZE, SECTION_DESCRIPTOR_SIZE,
};
use super::manifest::{AbstractActionPathsManifest, DrillScenariosManifest, ManifestFile};
use super::proto::{AbstractActionPathEntry, AbstractActionPathPage, DrillScenarioPage};
use super::source::{load_metadata, LoadedMetadata};

pub const DEFAULT_METADATA_PAGE_TARGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct MetadataExportOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub page_target_bytes: usize,
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedConcreteActionPath {
    pub source_id: u32,
    pub concrete_action_path_id: u32,
    pub abstract_action_path: String,
    pub concrete_action_path: String,
}

#[derive(Debug, Clone)]
pub struct MetadataExportSummary {
    pub drill_scenarios: DrillScenariosManifest,
    pub abstract_action_paths: AbstractActionPathsManifest,
    pub concrete_paths: Vec<ExportedConcreteActionPath>,
}

pub fn export_metadata(
    options: &MetadataExportOptions,
) -> Result<MetadataExportSummary, ToolError> {
    if !options.source_db.is_file() {
        return Err(ToolError::invalid_argument(format!(
            "Source database does not exist: {}",
            options.source_db.display()
        )));
    }
    if options.page_target_bytes == 0 {
        return Err(ToolError::invalid_argument(
            "V3 metadata page target must be positive",
        ));
    }
    let connection = Connection::open(&options.source_db, true)?;
    let loaded = load_metadata(&connection, &options.dimension)?;
    fs::create_dir_all(&options.out_dir)?;
    prepare_output_paths(&options.out_dir, options.overwrite)?;

    let paths = MetadataPaths::new(&options.out_dir);
    paths.remove_temporary_files();
    let result = write_metadata_to_temporary_files(&paths, &loaded, options.page_target_bytes);
    let (drill_scenarios, abstract_action_paths) = match result {
        Ok(manifests) => manifests,
        Err(error) => {
            paths.remove_temporary_files();
            return Err(error);
        }
    };
    if let Err(error) = paths.publish(options.overwrite) {
        paths.remove_temporary_files();
        return Err(error);
    }

    Ok(MetadataExportSummary {
        drill_scenarios,
        abstract_action_paths,
        concrete_paths: loaded
            .concrete_paths
            .into_iter()
            .map(|path| ExportedConcreteActionPath {
                source_id: path.source_id,
                concrete_action_path_id: path.concrete_action_path_id,
                abstract_action_path: path.abstract_action_path,
                concrete_action_path: path.concrete_action_path,
            })
            .collect(),
    })
}

pub struct MetadataStore {
    drill: PagedDataset,
    action_paths: PagedDataset,
}

impl MetadataStore {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ToolError> {
        let dir = dir.as_ref();
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
            let page: DrillScenarioPage = self.drill.read_page(locator.page_id)?;
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
            let page: AbstractActionPathPage = self.action_paths.read_page(locator.page_id)?;
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
            let page: AbstractActionPathPage = self.action_paths.read_page(locator.page_id)?;
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
}

struct MetadataPaths {
    final_paths: [PathBuf; 4],
    temporary_paths: [PathBuf; 4],
}

impl MetadataPaths {
    fn new(dir: &Path) -> Self {
        let final_paths = [
            dir.join(DRILL_SCENARIOS_DATA_FILE_NAME),
            dir.join(DRILL_SCENARIOS_INDEX_FILE_NAME),
            dir.join(ABSTRACT_ACTION_PATHS_DATA_FILE_NAME),
            dir.join(ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME),
        ];
        let temporary_paths = [
            dir.join(format!("{DRILL_SCENARIOS_DATA_FILE_NAME}.tmp")),
            dir.join(format!("{DRILL_SCENARIOS_INDEX_FILE_NAME}.tmp")),
            dir.join(format!("{ABSTRACT_ACTION_PATHS_DATA_FILE_NAME}.tmp")),
            dir.join(format!("{ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME}.tmp")),
        ];
        Self {
            final_paths,
            temporary_paths,
        }
    }

    fn remove_temporary_files(&self) {
        for path in &self.temporary_paths {
            let _ = fs::remove_file(path);
        }
    }

    fn publish(&self, overwrite: bool) -> Result<(), ToolError> {
        if overwrite {
            for path in &self.final_paths {
                if path.exists() {
                    fs::remove_file(path)?;
                }
            }
        }
        for (temporary, final_path) in self.temporary_paths.iter().zip(&self.final_paths) {
            fs::rename(temporary, final_path)?;
        }
        Ok(())
    }
}

fn prepare_output_paths(dir: &Path, overwrite: bool) -> Result<(), ToolError> {
    for file_name in [
        DRILL_SCENARIOS_DATA_FILE_NAME,
        DRILL_SCENARIOS_INDEX_FILE_NAME,
        ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
        ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
    ] {
        let path = dir.join(file_name);
        if path.exists() && !overwrite {
            return Err(ToolError::invalid_argument(format!(
                "V3 metadata output already exists: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_metadata_to_temporary_files(
    paths: &MetadataPaths,
    loaded: &LoadedMetadata,
    page_target_bytes: usize,
) -> Result<(DrillScenariosManifest, AbstractActionPathsManifest), ToolError> {
    let drill_pages = paginate(
        loaded.drill_scenarios.clone(),
        page_target_bytes,
        |entries| DrillScenarioPage { entries },
    );
    let action_path_pages = paginate(
        loaded.abstract_action_paths.clone(),
        page_target_bytes,
        |entries| AbstractActionPathPage { entries },
    );

    let drill_page_locators = write_page_data_file(
        &paths.temporary_paths[0],
        FileKind::DrillScenariosData,
        &drill_pages,
        loaded.drill_scenarios.len() as u64,
    )?;
    let mut drill_hash_locators = Vec::with_capacity(loaded.drill_scenarios.len());
    for (page_id, page) in drill_pages.iter().enumerate() {
        for (entry_index, entry) in page.entries.iter().enumerate() {
            drill_hash_locators.push(HashLocator::entry(
                stable_hash64(&entry.drill_name),
                to_u32("drill page id", page_id)?,
                to_u32("drill entry index", entry_index)?,
            ));
        }
    }
    sort_hash_locators(&mut drill_hash_locators);
    write_index_file(
        &paths.temporary_paths[1],
        FileKind::DrillScenariosIndex,
        drill_pages.len() as u64,
        drill_hash_locators.len() as u64,
        vec![
            payload_locator_section(&drill_page_locators),
            hash_locator_section(SectionKind::PrimaryHashLocators, &drill_hash_locators),
        ],
    )?;

    let action_page_locators = write_page_data_file(
        &paths.temporary_paths[2],
        FileKind::AbstractActionPathsData,
        &action_path_pages,
        loaded.abstract_action_paths.len() as u64,
    )?;
    let mut abstract_hash_locators = Vec::with_capacity(loaded.abstract_action_paths.len());
    let mut concrete_hash_locators = Vec::with_capacity(loaded.concrete_paths.len());
    for (page_id, page) in action_path_pages.iter().enumerate() {
        for (entry_index, entry) in page.entries.iter().enumerate() {
            let page_id = to_u32("action path page id", page_id)?;
            let entry_index = to_u32("action path entry index", entry_index)?;
            abstract_hash_locators.push(HashLocator::entry(
                stable_hash64(&entry.abstract_action_path),
                page_id,
                entry_index,
            ));
            for (value_index, path) in entry.concrete_action_paths.iter().enumerate() {
                concrete_hash_locators.push(HashLocator::value(
                    stable_hash64(&path.concrete_action_path),
                    page_id,
                    entry_index,
                    to_u32("concrete action path value index", value_index)?,
                ));
            }
        }
    }
    sort_hash_locators(&mut abstract_hash_locators);
    sort_hash_locators(&mut concrete_hash_locators);
    write_index_file(
        &paths.temporary_paths[3],
        FileKind::AbstractActionPathsIndex,
        action_path_pages.len() as u64,
        (abstract_hash_locators.len() + concrete_hash_locators.len()) as u64,
        vec![
            payload_locator_section(&action_page_locators),
            hash_locator_section(SectionKind::PrimaryHashLocators, &abstract_hash_locators),
            hash_locator_section(SectionKind::SecondaryHashLocators, &concrete_hash_locators),
        ],
    )?;

    let drill_scenarios = DrillScenariosManifest {
        data: manifest_file(
            &paths.temporary_paths[0],
            DRILL_SCENARIOS_DATA_FILE_NAME,
            drill_pages.len() as u64,
            loaded.drill_scenarios.len() as u64,
        )?,
        index: manifest_file(
            &paths.temporary_paths[1],
            DRILL_SCENARIOS_INDEX_FILE_NAME,
            drill_pages.len() as u64,
            drill_hash_locators.len() as u64,
        )?,
        page_count: drill_pages.len() as u64,
        drill_count: loaded.drill_scenarios.len() as u64,
        hash_record_count: drill_hash_locators.len() as u64,
    };
    let abstract_action_paths = AbstractActionPathsManifest {
        data: manifest_file(
            &paths.temporary_paths[2],
            ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
            action_path_pages.len() as u64,
            loaded.abstract_action_paths.len() as u64,
        )?,
        index: manifest_file(
            &paths.temporary_paths[3],
            ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
            action_path_pages.len() as u64,
            (abstract_hash_locators.len() + concrete_hash_locators.len()) as u64,
        )?,
        page_count: action_path_pages.len() as u64,
        abstract_path_count: loaded.abstract_action_paths.len() as u64,
        concrete_path_count: loaded.concrete_paths.len() as u64,
        abstract_hash_record_count: abstract_hash_locators.len() as u64,
        concrete_hash_record_count: concrete_hash_locators.len() as u64,
    };
    Ok((drill_scenarios, abstract_action_paths))
}

fn paginate<T, P, F>(entries: Vec<T>, page_target_bytes: usize, make_page: F) -> Vec<P>
where
    T: Clone,
    P: Message,
    F: Fn(Vec<T>) -> P,
{
    let mut pages = Vec::new();
    let mut current = Vec::new();
    for entry in entries {
        current.push(entry);
        if current.len() > 1 && make_page(current.clone()).encoded_len() > page_target_bytes {
            let overflow = current.pop().expect("page contains overflow entry");
            pages.push(make_page(std::mem::take(&mut current)));
            current.push(overflow);
        }
    }
    if !current.is_empty() {
        pages.push(make_page(current));
    }
    pages
}

fn write_page_data_file<P: Message>(
    path: &Path,
    kind: FileKind,
    pages: &[P],
    entry_count: u64,
) -> Result<Vec<PayloadLocator>, ToolError> {
    let mut file = File::create(path)?;
    file.write_all(&encode_header(FileHeader::new(
        kind,
        pages.len() as u64,
        entry_count,
        0,
    )))?;
    let mut offset = HEADER_SIZE as u64;
    let mut locators = Vec::with_capacity(pages.len());
    for page in pages {
        let payload = page.encode_to_vec();
        let byte_length = u32::try_from(payload.len()).map_err(|_| {
            ToolError::new(
                "V3_PROTO_PAGE_TOO_LARGE",
                "Encoded V3 metadata page exceeds uint32",
            )
        })?;
        file.write_all(&payload)?;
        locators.push(PayloadLocator {
            offset,
            byte_length,
            crc32c: crc32c(&payload),
        });
        offset = offset
            .checked_add(u64::from(byte_length))
            .ok_or_else(|| ToolError::invalid_format("V3 metadata data offset overflow"))?;
    }
    file.sync_all()?;
    Ok(locators)
}

struct EncodedSection {
    kind: SectionKind,
    record_size: u16,
    record_count: u64,
    bytes: Vec<u8>,
}

fn payload_locator_section(locators: &[PayloadLocator]) -> EncodedSection {
    let mut bytes = Vec::with_capacity(locators.len() * PAYLOAD_LOCATOR_SIZE);
    for locator in locators {
        bytes.extend_from_slice(&encode_payload_locator(*locator));
    }
    EncodedSection {
        kind: SectionKind::PageLocators,
        record_size: PAYLOAD_LOCATOR_SIZE as u16,
        record_count: locators.len() as u64,
        bytes,
    }
}

fn hash_locator_section(kind: SectionKind, locators: &[HashLocator]) -> EncodedSection {
    let mut bytes = Vec::with_capacity(locators.len() * HASH_LOCATOR_SIZE);
    for locator in locators {
        bytes.extend_from_slice(&encode_hash_locator(*locator));
    }
    EncodedSection {
        kind,
        record_size: HASH_LOCATOR_SIZE as u16,
        record_count: locators.len() as u64,
        bytes,
    }
}

fn write_index_file(
    path: &Path,
    kind: FileKind,
    primary_count: u64,
    secondary_count: u64,
    sections: Vec<EncodedSection>,
) -> Result<(), ToolError> {
    let section_count = u32::try_from(sections.len())
        .map_err(|_| ToolError::invalid_format("V3 index section count exceeds uint32"))?;
    let directory_bytes = sections
        .len()
        .checked_mul(SECTION_DESCRIPTOR_SIZE)
        .ok_or_else(|| ToolError::invalid_format("V3 section directory size overflow"))?;
    let mut offset = u64::try_from(HEADER_SIZE + directory_bytes)
        .map_err(|_| ToolError::invalid_format("V3 index section offset exceeds uint64"))?;
    let mut descriptors = Vec::with_capacity(sections.len());
    for section in &sections {
        if section.bytes.len()
            != usize::try_from(section.record_count)
                .ok()
                .and_then(|count| count.checked_mul(usize::from(section.record_size)))
                .ok_or_else(|| ToolError::invalid_format("V3 index section size overflow"))?
        {
            return Err(ToolError::invalid_format(
                "V3 encoded index section length does not match records",
            ));
        }
        let descriptor = SectionDescriptor::new(
            section.kind,
            section.record_size,
            offset,
            section.record_count,
        )?;
        offset = descriptor.end()?;
        descriptors.push(descriptor);
    }

    let mut file = File::create(path)?;
    file.write_all(&encode_header(FileHeader::new(
        kind,
        primary_count,
        secondary_count,
        section_count,
    )))?;
    for descriptor in descriptors {
        file.write_all(&encode_section_descriptor(descriptor))?;
    }
    for section in sections {
        file.write_all(&section.bytes)?;
    }
    file.sync_all()?;
    Ok(())
}

fn manifest_file(
    path: &Path,
    file_name: &str,
    primary_count: u64,
    secondary_count: u64,
) -> Result<ManifestFile, ToolError> {
    let bytes = fs::read(path)?;
    Ok(ManifestFile {
        file_name: file_name.to_owned(),
        size_bytes: bytes.len() as u64,
        crc32c: crc32c(&bytes),
        primary_count,
        secondary_count,
    })
}

fn sort_hash_locators(locators: &mut [HashLocator]) {
    locators.sort_unstable_by_key(|locator| {
        (
            locator.hash,
            locator.page_id,
            locator.entry_index,
            locator.value_index,
        )
    });
}

fn to_u32(name: &str, value: usize) -> Result<u32, ToolError> {
    u32::try_from(value)
        .map_err(|_| ToolError::new("V3_INDEX_VALUE_OVERFLOW", format!("{name} exceeds uint32")))
}

struct PagedDataset {
    data_mmap: Mmap,
    index_mmap: Mmap,
    _data_file: File,
    _index_file: File,
    page_locators: SectionView,
    primary_hash_locators: SectionView,
    secondary_hash_locators: Option<SectionView>,
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
        })
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
