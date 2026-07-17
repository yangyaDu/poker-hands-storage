use std::fs;
use std::path::{Path, PathBuf};

use prost::Message;
use range_store_core::dimension::DimensionSpec;
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

use super::format::{
    checked_u32_index, encode_hash_locator, sort_hash_locators, stable_hash64, FileKind,
    HashLocator, SectionKind, ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
    ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME, DRILL_SCENARIOS_DATA_FILE_NAME,
    DRILL_SCENARIOS_INDEX_FILE_NAME, HASH_LOCATOR_SIZE,
};
use super::manifest::{AbstractActionPathsManifest, DrillScenariosManifest};
use super::proto::{AbstractActionPathPage, DrillScenarioPage};
use super::source::load_metadata;
use super::source::LoadedMetadata;
use super::storage_file::{
    manifest_file, payload_locator_section, write_index_file, write_payload_data_file,
    EncodedSection, StagedFilePair,
};

/// Preferred upper bound for an individual metadata protobuf page.
pub const DEFAULT_METADATA_PAGE_TARGET_BYTES: usize = 64 * 1024;

/// Inputs for exporting the dimension-local metadata datasets.
#[derive(Debug, Clone)]
pub struct MetadataExportOptions {
    pub source_db: PathBuf,
    pub out_dir: PathBuf,
    pub dimension: DimensionSpec,
    pub page_target_bytes: usize,
    pub overwrite: bool,
}

/// Mapping from the source SQLite id to the dense id used in a V3 archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedConcreteActionPath {
    pub source_id: u32,
    pub concrete_action_path_id: u32,
    pub abstract_action_path: String,
    pub concrete_action_path: String,
}

/// Metadata files and concrete-path mapping produced for one V3 dimension.
#[derive(Debug, Clone)]
pub struct MetadataExportSummary {
    pub drill_scenarios: DrillScenariosManifest,
    pub abstract_action_paths: AbstractActionPathsManifest,
    pub concrete_paths: Vec<ExportedConcreteActionPath>,
}

/// Exports the metadata needed by both V3 lookup datasets and hand strategies.
///
/// This is the export seam: callers provide a source database and a target dimension, while
/// loading, pagination, index construction, temporary files, and publication remain internal.
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

struct MetadataPaths {
    drill_scenarios: StagedFilePair,
    abstract_action_paths: StagedFilePair,
}

impl MetadataPaths {
    fn new(dir: &Path) -> Self {
        Self {
            drill_scenarios: StagedFilePair::new(
                dir,
                DRILL_SCENARIOS_DATA_FILE_NAME,
                DRILL_SCENARIOS_INDEX_FILE_NAME,
            ),
            abstract_action_paths: StagedFilePair::new(
                dir,
                ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
                ABSTRACT_ACTION_PATHS_INDEX_FILE_NAME,
            ),
        }
    }

    fn remove_temporary_files(&self) {
        self.drill_scenarios.remove_temporary_files();
        self.abstract_action_paths.remove_temporary_files();
    }

    fn publish(&self, overwrite: bool) -> Result<(), ToolError> {
        if overwrite {
            self.drill_scenarios.remove_final_files()?;
            self.abstract_action_paths.remove_final_files()?;
        }
        self.drill_scenarios.publish_temporary_files()?;
        self.abstract_action_paths.publish_temporary_files()?;
        Ok(())
    }
}

fn prepare_output_paths(dir: &Path, overwrite: bool) -> Result<(), ToolError> {
    let paths = MetadataPaths::new(dir);
    for path in paths
        .drill_scenarios
        .final_paths()
        .into_iter()
        .chain(paths.abstract_action_paths.final_paths())
    {
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

    let drill_page_locators = write_payload_data_file(
        &paths.drill_scenarios.data.temporary_path,
        FileKind::DrillScenariosData,
        drill_pages.len(),
        loaded.drill_scenarios.len() as u64,
        |page_index| Ok(drill_pages[page_index].encode_to_vec()),
        || {
            ToolError::new(
                "V3_PROTO_PAGE_TOO_LARGE",
                "Encoded V3 metadata page exceeds uint32",
            )
        },
        || ToolError::invalid_format("V3 metadata data offset overflow"),
    )?;
    let mut drill_hash_locators = Vec::with_capacity(loaded.drill_scenarios.len());
    for (page_id, page) in drill_pages.iter().enumerate() {
        for (entry_index, entry) in page.entries.iter().enumerate() {
            drill_hash_locators.push(HashLocator::entry(
                stable_hash64(&entry.drill_name),
                checked_u32_index("drill page id", page_id)?,
                checked_u32_index("drill entry index", entry_index)?,
            ));
        }
    }
    sort_hash_locators(&mut drill_hash_locators);
    write_index_file(
        &paths.drill_scenarios.index.temporary_path,
        FileKind::DrillScenariosIndex,
        drill_pages.len() as u64,
        drill_hash_locators.len() as u64,
        vec![
            payload_locator_section(SectionKind::PageLocators, &drill_page_locators),
            hash_locator_section(SectionKind::PrimaryHashLocators, &drill_hash_locators),
        ],
    )?;

    let action_page_locators = write_payload_data_file(
        &paths.abstract_action_paths.data.temporary_path,
        FileKind::AbstractActionPathsData,
        action_path_pages.len(),
        loaded.abstract_action_paths.len() as u64,
        |page_index| Ok(action_path_pages[page_index].encode_to_vec()),
        || {
            ToolError::new(
                "V3_PROTO_PAGE_TOO_LARGE",
                "Encoded V3 metadata page exceeds uint32",
            )
        },
        || ToolError::invalid_format("V3 metadata data offset overflow"),
    )?;
    let mut abstract_hash_locators = Vec::with_capacity(loaded.abstract_action_paths.len());
    let mut concrete_hash_locators = Vec::with_capacity(loaded.concrete_paths.len());
    for (page_id, page) in action_path_pages.iter().enumerate() {
        for (entry_index, entry) in page.entries.iter().enumerate() {
            let page_id = checked_u32_index("action path page id", page_id)?;
            let entry_index = checked_u32_index("action path entry index", entry_index)?;
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
                    checked_u32_index("concrete action path value index", value_index)?,
                ));
            }
        }
    }
    sort_hash_locators(&mut abstract_hash_locators);
    sort_hash_locators(&mut concrete_hash_locators);
    write_index_file(
        &paths.abstract_action_paths.index.temporary_path,
        FileKind::AbstractActionPathsIndex,
        action_path_pages.len() as u64,
        (abstract_hash_locators.len() + concrete_hash_locators.len()) as u64,
        vec![
            payload_locator_section(SectionKind::PageLocators, &action_page_locators),
            hash_locator_section(SectionKind::PrimaryHashLocators, &abstract_hash_locators),
            hash_locator_section(SectionKind::SecondaryHashLocators, &concrete_hash_locators),
        ],
    )?;

    let drill_scenarios = DrillScenariosManifest {
        data: manifest_file(
            &paths.drill_scenarios.data.temporary_path,
            DRILL_SCENARIOS_DATA_FILE_NAME,
            drill_pages.len() as u64,
            loaded.drill_scenarios.len() as u64,
        )?,
        index: manifest_file(
            &paths.drill_scenarios.index.temporary_path,
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
            &paths.abstract_action_paths.data.temporary_path,
            ABSTRACT_ACTION_PATHS_DATA_FILE_NAME,
            action_path_pages.len() as u64,
            loaded.abstract_action_paths.len() as u64,
        )?,
        index: manifest_file(
            &paths.abstract_action_paths.index.temporary_path,
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
