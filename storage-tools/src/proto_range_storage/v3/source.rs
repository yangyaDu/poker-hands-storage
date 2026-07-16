use std::collections::{BTreeMap, HashMap, HashSet};

use range_store_core::dimension::{get_drill_scenario_table_name, quote_identifier, DimensionSpec};
use range_store_core::sqlite::{Connection, Value};

use crate::errors::ToolError;

use super::proto::{AbstractActionPathEntry, ConcreteActionPathRef, DrillScenarioEntry};

#[derive(Debug, Clone)]
pub(crate) struct ConcretePathMapping {
    pub source_id: u32,
    pub concrete_action_path_id: u32,
    pub abstract_action_path: String,
    pub concrete_action_path: String,
}

#[derive(Debug)]
pub(crate) struct LoadedMetadata {
    pub drill_scenarios: Vec<DrillScenarioEntry>,
    pub abstract_action_paths: Vec<AbstractActionPathEntry>,
    pub concrete_paths: Vec<ConcretePathMapping>,
}

pub(crate) fn load_metadata(
    connection: &Connection,
    dimension: &DimensionSpec,
) -> Result<LoadedMetadata, ToolError> {
    let concrete_paths = load_concrete_paths(connection, dimension)?;
    let mut paths_by_abstract = BTreeMap::<String, Vec<ConcreteActionPathRef>>::new();
    for path in &concrete_paths {
        paths_by_abstract
            .entry(path.abstract_action_path.clone())
            .or_default()
            .push(ConcreteActionPathRef {
                concrete_action_path_id: path.concrete_action_path_id,
                concrete_action_path: path.concrete_action_path.clone(),
            });
    }
    let abstract_action_paths = paths_by_abstract
        .into_iter()
        .map(
            |(abstract_action_path, concrete_action_paths)| AbstractActionPathEntry {
                abstract_action_path,
                concrete_action_paths,
            },
        )
        .collect::<Vec<_>>();
    let abstract_paths = abstract_action_paths
        .iter()
        .map(|entry| entry.abstract_action_path.as_str())
        .collect::<HashSet<_>>();
    let drill_scenarios = load_drill_scenarios(connection, dimension)?;
    for drill in &drill_scenarios {
        for abstract_path in &drill.abstract_action_paths {
            if !abstract_paths.contains(abstract_path.as_str()) {
                return Err(ToolError::new(
                    "V3_DRILL_ABSTRACT_PATH_NOT_FOUND",
                    format!(
                        "Drill {:?} references abstract action path {:?} outside dimension {}:{}:{}",
                        drill.drill_name,
                        abstract_path,
                        dimension.strategy,
                        dimension.player_count,
                        dimension.depth_bb
                    ),
                ));
            }
        }
    }
    Ok(LoadedMetadata {
        drill_scenarios,
        abstract_action_paths,
        concrete_paths,
    })
}

fn load_concrete_paths(
    connection: &Connection,
    dimension: &DimensionSpec,
) -> Result<Vec<ConcretePathMapping>, ToolError> {
    let table = quote_identifier(&dimension.concrete_table())?;
    let mut statement = connection.prepare(&format!(
        "SELECT id, abstract_line, concrete_line FROM {table} ORDER BY id"
    ))?;
    statement.start(&[])?;

    let mut source_rows = Vec::new();
    let mut seen_source_ids = HashSet::new();
    let mut seen_concrete_paths = HashMap::<String, u32>::new();
    while statement.step_row()? {
        let source_id = statement.column_u32(0)?;
        let abstract_action_path = statement.column_text(1)?;
        let concrete_action_path = statement.column_text(2)?;
        if source_id == 0 || !seen_source_ids.insert(source_id) {
            return Err(ToolError::new(
                "INVALID_SOURCE_CONCRETE_ACTION_PATH_ID",
                format!("Source concrete action path id {source_id} is invalid or duplicated"),
            ));
        }
        if let Some(previous_source_id) =
            seen_concrete_paths.insert(concrete_action_path.clone(), source_id)
        {
            return Err(ToolError::new(
                "DUPLICATE_CONCRETE_ACTION_PATH",
                format!(
                    "Concrete action path {concrete_action_path:?} appears at source ids {previous_source_id} and {source_id}"
                ),
            ));
        }
        source_rows.push((source_id, abstract_action_path, concrete_action_path));
    }
    if source_rows.is_empty() {
        return Err(ToolError::new(
            "V3_ACTION_PATHS_EMPTY",
            "The selected dimension has no concrete action paths",
        ));
    }

    source_rows
        .into_iter()
        .enumerate()
        .map(
            |(index, (source_id, abstract_action_path, concrete_action_path))| {
                let concrete_action_path_id = u32::try_from(index + 1).map_err(|_| {
                    ToolError::new(
                        "V3_CONCRETE_ACTION_PATH_ID_OVERFLOW",
                        "Concrete action path count exceeds uint32",
                    )
                })?;
                Ok(ConcretePathMapping {
                    source_id,
                    concrete_action_path_id,
                    abstract_action_path,
                    concrete_action_path,
                })
            },
        )
        .collect()
}

fn load_drill_scenarios(
    connection: &Connection,
    dimension: &DimensionSpec,
) -> Result<Vec<DrillScenarioEntry>, ToolError> {
    let raw_table = get_drill_scenario_table_name(&dimension.strategy);
    let table = quote_identifier(&raw_table)?;
    let depth_column = find_drill_depth_column(connection, &table)?;
    let mut statement = connection.prepare(&format!(
        "SELECT drill_name, abstract_line
         FROM {table}
         WHERE player_count = ?1 AND {depth_column} = ?2
         ORDER BY drill_name, abstract_line"
    ))?;
    statement.start(&[
        Value::from(dimension.player_count),
        Value::from(dimension.depth_bb),
    ])?;

    let mut drills = BTreeMap::<String, Vec<String>>::new();
    while statement.step_row()? {
        let drill_name = statement.column_text(0)?;
        let abstract_path = statement.column_text(1)?;
        let paths = drills.entry(drill_name.clone()).or_default();
        if paths.last() == Some(&abstract_path) {
            return Err(ToolError::new(
                "DUPLICATE_DRILL_ABSTRACT_ACTION_PATH",
                format!(
                    "Drill {drill_name:?} contains duplicate abstract action path {abstract_path:?}"
                ),
            ));
        }
        paths.push(abstract_path);
    }
    if drills.is_empty() {
        return Err(ToolError::new(
            "V3_DRILL_SCENARIOS_EMPTY",
            format!(
                "No drill scenarios found for dimension {}:{}:{}",
                dimension.strategy, dimension.player_count, dimension.depth_bb
            ),
        ));
    }
    Ok(drills
        .into_iter()
        .map(|(drill_name, abstract_action_paths)| DrillScenarioEntry {
            drill_name,
            abstract_action_paths,
        })
        .collect())
}

fn find_drill_depth_column(
    connection: &Connection,
    quoted_table: &str,
) -> Result<String, ToolError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({quoted_table})"))?;
    statement.start(&[])?;
    let mut has_depth = false;
    let mut has_drill_depth = false;
    while statement.step_row()? {
        match statement.column_text(1)?.as_str() {
            "depth" => has_depth = true,
            "drill_depth" => has_drill_depth = true,
            _ => {}
        }
    }
    let column = if has_depth {
        "depth"
    } else if has_drill_depth {
        "drill_depth"
    } else {
        return Err(ToolError::new(
            "INVALID_SOURCE_DRILL_SCHEMA",
            "Drill scenario table must contain depth or drill_depth",
        ));
    };
    Ok(quote_identifier(column)?)
}
