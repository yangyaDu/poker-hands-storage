use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use range_store_core::dimension::DimensionSpec;
use range_store_core::hole_cards::hand_code_from_id;
use range_store_core::sqlite::Connection;
use serde::Serialize;

use crate::errors::ToolError;

use super::archive::{V3Archive, V3ArchiveOpenOptions};
use super::metadata_store::MetadataSnapshot;
use super::proto::{ActionStrategyColumn, HandStrategy};
use super::source::{load_metadata, load_strategy_rows, LoadedMetadata};
use super::strategy_codec::{build_hand_strategy, DecodedHandStrategy, HAND_COUNT_PREFLOP};

#[derive(Debug, Clone, Copy)]
pub struct V3VerificationOptions {
    pub max_failure_samples: usize,
}

impl Default for V3VerificationOptions {
    fn default() -> Self {
        Self {
            max_failure_samples: 50,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3VerificationCounts {
    pub files_checked: u64,
    pub drill_scenarios: u64,
    pub abstract_action_paths: u64,
    pub concrete_action_paths: u64,
    pub hand_strategies: u64,
    pub hands_visited: u64,
    pub action_cells_compared: u64,
    pub source_action_cells: u64,
    pub null_ev_cells: u64,
    pub mapping_differences: u64,
    pub action_differences: u64,
    pub cell_differences: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3VerificationFailure {
    pub code: String,
    pub context: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V3VerificationReport {
    pub mode: String,
    pub ok: bool,
    pub archive_dir: PathBuf,
    pub source_db: Option<PathBuf>,
    pub elapsed_ms: u128,
    pub failure_count: u64,
    pub counts: V3VerificationCounts,
    pub failure_samples: Vec<V3VerificationFailure>,
}

impl V3VerificationReport {
    fn new(mode: &str, archive_dir: &Path, source_db: Option<&Path>) -> Self {
        Self {
            mode: mode.to_owned(),
            ok: false,
            archive_dir: archive_dir.to_path_buf(),
            source_db: source_db.map(Path::to_path_buf),
            elapsed_ms: 0,
            failure_count: 0,
            counts: V3VerificationCounts::default(),
            failure_samples: Vec::new(),
        }
    }

    fn fail(
        &mut self,
        options: V3VerificationOptions,
        code: impl Into<String>,
        context: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.failure_count = self.failure_count.saturating_add(1);
        if self.failure_samples.len() < options.max_failure_samples {
            self.failure_samples.push(V3VerificationFailure {
                code: code.into(),
                context: context.into(),
                message: message.into(),
            });
        }
    }

    fn fail_error(
        &mut self,
        options: V3VerificationOptions,
        context: impl Into<String>,
        error: &ToolError,
    ) {
        self.fail(options, error.code(), context, error.message());
    }

    fn finish(&mut self, started: Instant) {
        self.ok = self.failure_count == 0;
        self.elapsed_ms = started.elapsed().as_millis();
    }
}

struct VerifiedArchive {
    archive: V3Archive,
    metadata: MetadataSnapshot,
    strategies: Vec<HandStrategy>,
}

pub fn verify_v3_archive(
    archive_dir: impl AsRef<Path>,
    options: V3VerificationOptions,
) -> V3VerificationReport {
    let archive_dir = archive_dir.as_ref();
    let started = Instant::now();
    let mut report = V3VerificationReport::new("standalone", archive_dir, None);
    match load_verified_archive(archive_dir, &mut report, options) {
        Ok(_) => {}
        Err(error) => report.fail_error(options, "archive", &error),
    }
    report.finish(started);
    report
}

pub fn cross_verify_sqlite_v3(
    source_db: impl AsRef<Path>,
    archive_dir: impl AsRef<Path>,
    options: V3VerificationOptions,
) -> V3VerificationReport {
    let source_db = source_db.as_ref();
    let archive_dir = archive_dir.as_ref();
    let started = Instant::now();
    let mut report = V3VerificationReport::new("sqlite-v3", archive_dir, Some(source_db));
    let verified = match load_verified_archive(archive_dir, &mut report, options) {
        Ok(verified) => verified,
        Err(error) => {
            report.fail_error(options, "archive", &error);
            report.finish(started);
            return report;
        }
    };
    let connection = match Connection::open(source_db, true) {
        Ok(connection) => connection,
        Err(error) => {
            report.fail_error(options, "source_db", &error.into());
            report.finish(started);
            return report;
        }
    };
    let dimension = verified.archive.dimension();
    let source = match load_metadata(&connection, &dimension) {
        Ok(source) => source,
        Err(error) => {
            report.fail_error(options, "source_metadata", &error);
            report.finish(started);
            return report;
        }
    };

    compare_metadata(&source, &verified.metadata, &mut report, options);
    compare_strategies(
        &connection,
        &dimension,
        &source,
        &verified.metadata,
        &verified.strategies,
        &mut report,
        options,
    );
    report.finish(started);
    report
}

fn load_verified_archive(
    archive_dir: &Path,
    report: &mut V3VerificationReport,
    options: V3VerificationOptions,
) -> Result<VerifiedArchive, ToolError> {
    let archive = V3Archive::open_with_options(
        archive_dir,
        V3ArchiveOpenOptions {
            verify_file_checksums: true,
            metadata_cache_byte_budget: 0,
            strategy_cache_byte_budget: 0,
        },
    )?;
    let metadata = archive.metadata().verify_and_snapshot()?;
    let strategies = archive.strategies().verify_and_snapshot()?;
    let manifest = archive.manifest();

    report.counts.files_checked = 6;
    report.counts.drill_scenarios = metadata.drill_scenarios.len() as u64;
    report.counts.abstract_action_paths = metadata.abstract_action_paths.len() as u64;
    report.counts.concrete_action_paths = metadata.concrete_path_count;
    report.counts.hand_strategies = strategies.len() as u64;

    check_count(
        report,
        options,
        "drill_scenarios",
        manifest.drill_scenarios.drill_count,
        report.counts.drill_scenarios,
    );
    check_count(
        report,
        options,
        "abstract_action_paths",
        manifest.abstract_action_paths.abstract_path_count,
        report.counts.abstract_action_paths,
    );
    check_count(
        report,
        options,
        "concrete_action_paths",
        manifest.abstract_action_paths.concrete_path_count,
        report.counts.concrete_action_paths,
    );
    check_count(
        report,
        options,
        "hand_strategies",
        manifest.hand_strategies.record_count,
        report.counts.hand_strategies,
    );
    if metadata.concrete_path_count != strategies.len() as u64 {
        report.fail(
            options,
            "V3_REFERENCE_COUNT_MISMATCH",
            "archive",
            format!(
                "{} concrete path refs but {} strategy records",
                metadata.concrete_path_count,
                strategies.len()
            ),
        );
    }
    Ok(VerifiedArchive {
        archive,
        metadata,
        strategies,
    })
}

fn check_count(
    report: &mut V3VerificationReport,
    options: V3VerificationOptions,
    context: &str,
    manifest_count: u64,
    decoded_count: u64,
) {
    if manifest_count != decoded_count {
        report.fail(
            options,
            "V3_MANIFEST_COUNT_MISMATCH",
            context,
            format!("manifest={manifest_count}, decoded={decoded_count}"),
        );
    }
}

fn compare_metadata(
    source: &LoadedMetadata,
    actual: &MetadataSnapshot,
    report: &mut V3VerificationReport,
    options: V3VerificationOptions,
) {
    let expected_drills = source
        .drill_scenarios
        .iter()
        .map(|entry| (&entry.drill_name, &entry.abstract_action_paths))
        .collect::<BTreeMap<_, _>>();
    let actual_drills = actual
        .drill_scenarios
        .iter()
        .map(|entry| (&entry.drill_name, &entry.abstract_action_paths))
        .collect::<BTreeMap<_, _>>();
    for name in expected_drills
        .keys()
        .chain(actual_drills.keys())
        .collect::<BTreeSet<_>>()
    {
        if expected_drills.get(name) != actual_drills.get(name) {
            report.counts.mapping_differences += 1;
            report.fail(
                options,
                "V3_DRILL_MAPPING_MISMATCH",
                format!("drill={name}"),
                format!(
                    "sqlite={:?}, v3={:?}",
                    expected_drills.get(name),
                    actual_drills.get(name)
                ),
            );
        }
    }

    let expected_abstract = source
        .abstract_action_paths
        .iter()
        .map(|entry| (&entry.abstract_action_path, &entry.concrete_action_paths))
        .collect::<BTreeMap<_, _>>();
    let actual_abstract = actual
        .abstract_action_paths
        .iter()
        .map(|entry| (&entry.abstract_action_path, &entry.concrete_action_paths))
        .collect::<BTreeMap<_, _>>();
    for path in expected_abstract
        .keys()
        .chain(actual_abstract.keys())
        .collect::<BTreeSet<_>>()
    {
        if expected_abstract.get(path) != actual_abstract.get(path) {
            report.counts.mapping_differences += 1;
            report.fail(
                options,
                "V3_ABSTRACT_MAPPING_MISMATCH",
                format!("abstract_path={path}"),
                format!(
                    "sqlite={:?}, v3={:?}",
                    expected_abstract.get(path),
                    actual_abstract.get(path)
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_strategies(
    connection: &Connection,
    dimension: &DimensionSpec,
    source: &LoadedMetadata,
    actual_metadata: &MetadataSnapshot,
    actual_strategies: &[HandStrategy],
    report: &mut V3VerificationReport,
    options: V3VerificationOptions,
) {
    let actual_paths = actual_metadata
        .abstract_action_paths
        .iter()
        .flat_map(|entry| {
            entry.concrete_action_paths.iter().map(move |path| {
                (
                    path.concrete_action_path.as_str(),
                    (
                        entry.abstract_action_path.as_str(),
                        path.concrete_action_path_id,
                    ),
                )
            })
        })
        .collect::<HashMap<_, _>>();

    for expected_path in &source.concrete_paths {
        let context = format!("concrete_path={}", expected_path.concrete_action_path);
        let Some(&(actual_abstract_path, actual_id)) =
            actual_paths.get(expected_path.concrete_action_path.as_str())
        else {
            report.counts.mapping_differences += 1;
            report.fail(
                options,
                "V3_CONCRETE_MAPPING_MISMATCH",
                context,
                "Concrete action path is missing from V3",
            );
            continue;
        };
        if actual_abstract_path != expected_path.abstract_action_path
            || actual_id != expected_path.concrete_action_path_id
        {
            report.counts.mapping_differences += 1;
            report.fail(
                options,
                "V3_CONCRETE_MAPPING_MISMATCH",
                &context,
                format!(
                    "sqlite=({}, {}), v3=({}, {})",
                    expected_path.abstract_action_path,
                    expected_path.concrete_action_path_id,
                    actual_abstract_path,
                    actual_id
                ),
            );
        }
        let rows = match load_strategy_rows(connection, dimension, expected_path.source_id) {
            Ok(rows) => rows,
            Err(error) => {
                report.fail_error(options, &context, &error);
                continue;
            }
        };
        let expected = match build_hand_strategy(&rows) {
            Ok(strategy) => strategy,
            Err(error) => {
                report.fail_error(options, &context, &error);
                continue;
            }
        };
        let Some(actual) = actual_id
            .checked_sub(1)
            .and_then(|index| actual_strategies.get(index as usize))
        else {
            report.fail(
                options,
                "V3_STRATEGY_REFERENCE_MISSING",
                context,
                format!("No strategy record for V3 id {actual_id}"),
            );
            continue;
        };
        compare_strategy(&context, &expected, actual, report, options);
    }
}

fn compare_strategy(
    context: &str,
    expected: &HandStrategy,
    actual: &HandStrategy,
    report: &mut V3VerificationReport,
    options: V3VerificationOptions,
) {
    let expected_decoded = DecodedHandStrategy::new(expected.clone())
        .expect("SQLite strategy was validated while being built");
    let actual_decoded = DecodedHandStrategy::new(actual.clone())
        .expect("V3 strategy was validated by standalone verification");
    let expected_actions = action_indexes(expected);
    let actual_actions = action_indexes(actual);
    let identities = expected_actions
        .keys()
        .chain(actual_actions.keys())
        .copied()
        .collect::<BTreeSet<_>>();

    report.counts.hands_visited += HAND_COUNT_PREFLOP as u64;
    for identity in identities {
        let (Some(&expected_index), Some(&actual_index)) = (
            expected_actions.get(&identity),
            actual_actions.get(&identity),
        ) else {
            report.counts.action_differences += 1;
            report.fail(
                options,
                "V3_ACTION_IDENTITY_MISMATCH",
                context,
                format!(
                    "action={identity:?}, sqlite_present={}, v3_present={}",
                    expected_actions.contains_key(&identity),
                    actual_actions.contains_key(&identity)
                ),
            );
            continue;
        };
        for hand_id in 0..HAND_COUNT_PREFLOP {
            report.counts.action_cells_compared += 1;
            let expected_value = expected_decoded.action_value(expected_index, hand_id);
            let actual_value = actual_decoded.action_value(actual_index, hand_id);
            if let Some(value) = expected_value {
                report.counts.source_action_cells += 1;
                if value.hand_ev_is_null {
                    report.counts.null_ev_cells += 1;
                }
            }
            if expected_value != actual_value {
                report.counts.cell_differences += 1;
                let code = if expected_value.map(|value| value.hand_ev_is_null)
                    != actual_value.map(|value| value.hand_ev_is_null)
                {
                    "V3_NULL_EV_MISMATCH"
                } else {
                    "V3_ACTION_CELL_MISMATCH"
                };
                report.fail(
                    options,
                    code,
                    format!(
                        "{context} hand={} action={identity:?}",
                        hand_code_from_id(hand_id as u8)
                    ),
                    format!("sqlite={expected_value:?}, v3={actual_value:?}"),
                );
            }
        }
    }
}

type ActionIdentity = (i32, u32, u32);

fn action_indexes(strategy: &HandStrategy) -> BTreeMap<ActionIdentity, usize> {
    strategy
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| (action_identity(action), index))
        .collect()
}

fn action_identity(action: &ActionStrategyColumn) -> ActionIdentity {
    (
        action.action_type,
        action.action_size_x10000,
        action.amount_centi_bb,
    )
}
