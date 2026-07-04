use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::action_schema::{decode_action_blob, ActionDef, ActionSchemaError};
use crate::dimension::{
    get_concrete_lines_table_name, get_drill_scenario_table_name, quote_identifier, NamingError,
};
use crate::sqlite::{Connection, SqliteError, Value};

#[derive(Debug, Clone)]
pub struct MetadataReader {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ConcreteLineRow {
    /// Concrete line id used by range queries.
    pub concrete_line_id: u32,
    /// Abstract action line.
    pub abstract_line: String,
    /// Concrete action line.
    pub concrete_line: String,
}

#[derive(Debug, Clone, Copy)]
pub enum ConcreteLineFilter<'a> {
    Abstract(&'a str),
    Concrete(&'a str),
    AbstractAndConcrete {
        abstract_line: &'a str,
        concrete_line: &'a str,
    },
}

#[derive(Debug)]
pub enum MetadataError {
    Sqlite(SqliteError),
    Naming(NamingError),
    ActionSchema(ActionSchemaError),
    ActionSchemaNotFound(u32),
    AbstractLineNotFound {
        strategy: String,
        player_count: u32,
        depth_bb: u32,
        abstract_line: String,
    },
    ConcreteLineValueNotFound {
        strategy: String,
        player_count: u32,
        depth_bb: u32,
        concrete_line: String,
    },
    ConcreteLineFilterNotFound {
        strategy: String,
        player_count: u32,
        depth_bb: u32,
        abstract_line: String,
        concrete_line: String,
    },
    DrillScenarioNotFound {
        strategy: String,
        drill_name: String,
        player_count: u32,
        drill_depth: u32,
    },
}

impl MetadataReader {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_action_schemas(&self) -> Result<HashMap<u32, Vec<ActionDef>>, MetadataError> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT id, action_count, action_blob FROM action_schemas ORDER BY id")?;
        statement.start(&[])?;
        let mut schemas = HashMap::new();
        while statement.step_row()? {
            let id = statement.column_u32(0)?;
            let action_count = statement.column_u32(1)?;
            let action_blob = statement.column_blob(2);
            schemas.insert(id, decode_action_blob(&action_blob, action_count)?);
        }
        Ok(schemas)
    }

    pub fn validate_dimension_schema_refs(
        &self,
        action_schema_ids: &HashSet<u32>,
    ) -> Result<(), MetadataError> {
        let connection = self.open()?;
        let mut statement =
            connection.prepare("SELECT DISTINCT action_schema_id FROM dimension_action_schemas")?;
        statement.start(&[])?;
        while statement.step_row()? {
            let action_schema_id = statement.column_u32(0)?;
            if !action_schema_ids.contains(&action_schema_id) {
                return Err(MetadataError::ActionSchemaNotFound(action_schema_id));
            }
        }
        Ok(())
    }

    pub fn dimension_action_schema_ids(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Result<Vec<u32>, MetadataError> {
        let connection = self.open()?;
        let mut statement = connection.prepare(
            "SELECT action_schema_id
             FROM dimension_action_schemas
             WHERE strategy = ?1 AND player_count = ?2 AND depth_bb = ?3
             ORDER BY action_schema_id",
        )?;
        statement.start(&[
            Value::from(strategy),
            Value::from(player_count),
            Value::from(depth_bb),
        ])?;
        let mut ids = Vec::new();
        while statement.step_row()? {
            ids.push(statement.column_u32(0)?);
        }
        Ok(ids)
    }

    pub fn get_concrete_lines(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        filter: ConcreteLineFilter<'_>,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError> {
        let table = quote_identifier(&get_concrete_lines_table_name(
            strategy,
            player_count,
            depth_bb,
        ))?;
        let connection = self.open()?;
        let (where_clause, values) = match filter {
            ConcreteLineFilter::Abstract(abstract_line) => {
                ("abstract_line = ?1", vec![Value::from(abstract_line)])
            }
            ConcreteLineFilter::Concrete(concrete_line) => {
                ("concrete_line = ?1", vec![Value::from(concrete_line)])
            }
            ConcreteLineFilter::AbstractAndConcrete {
                abstract_line,
                concrete_line,
            } => (
                "abstract_line = ?1 AND concrete_line = ?2",
                vec![Value::from(abstract_line), Value::from(concrete_line)],
            ),
        };
        let sql = format!(
            "SELECT concrete_line_id, abstract_line, concrete_line
             FROM {table}
             WHERE {where_clause}
             ORDER BY concrete_line_id"
        );
        let mut statement = connection.prepare(&sql).map_err(|_| {
            concrete_line_filter_not_found(strategy, player_count, depth_bb, filter)
        })?;
        statement.start(&values)?;
        let mut lines = Vec::new();
        while statement.step_row()? {
            lines.push(ConcreteLineRow {
                concrete_line_id: statement.column_u32(0)?,
                abstract_line: statement.column_text(1)?,
                concrete_line: statement.column_text(2)?,
            });
        }
        if lines.is_empty() {
            return Err(concrete_line_filter_not_found(
                strategy,
                player_count,
                depth_bb,
                filter,
            ));
        }
        Ok(lines)
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, MetadataError> {
        let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
        let connection = self.open()?;
        let sql = format!(
            "SELECT abstract_line
             FROM {table}
             WHERE drill_name = ?1 AND player_count = ?2 AND drill_depth = ?3
             ORDER BY abstract_line"
        );
        let mut statement =
            connection
                .prepare(&sql)
                .map_err(|_| MetadataError::DrillScenarioNotFound {
                    strategy: strategy.to_owned(),
                    drill_name: drill_name.to_owned(),
                    player_count,
                    drill_depth,
                })?;
        statement.start(&[
            Value::from(drill_name),
            Value::from(player_count),
            Value::from(drill_depth),
        ])?;
        let mut lines = Vec::new();
        while statement.step_row()? {
            lines.push(statement.column_text(0)?);
        }
        if lines.is_empty() {
            return Err(MetadataError::DrillScenarioNotFound {
                strategy: strategy.to_owned(),
                drill_name: drill_name.to_owned(),
                player_count,
                drill_depth,
            });
        }
        Ok(lines)
    }

    fn open(&self) -> Result<Connection, MetadataError> {
        Ok(Connection::open(&self.path, true)?)
    }
}

impl MetadataError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "META_DB_ERROR",
            Self::Naming(_) => "INVALID_ARGUMENT",
            Self::ActionSchema(_) => "INVALID_FORMAT",
            Self::ActionSchemaNotFound(_) => "ACTION_SCHEMA_NOT_FOUND",
            Self::AbstractLineNotFound { .. } => "ABSTRACT_LINE_NOT_FOUND",
            Self::ConcreteLineValueNotFound { .. } | Self::ConcreteLineFilterNotFound { .. } => {
                "CONCRETE_LINE_NOT_FOUND"
            }
            Self::DrillScenarioNotFound { .. } => "DRILL_SCENARIO_NOT_FOUND",
        }
    }

    pub fn public_code(&self) -> i32 {
        match self.code() {
            "INVALID_ARGUMENT" => 1000,
            "ACTION_SCHEMA_NOT_FOUND"
            | "ABSTRACT_LINE_NOT_FOUND"
            | "CONCRETE_LINE_NOT_FOUND"
            | "DRILL_SCENARIO_NOT_FOUND" => 404,
            _ => 500,
        }
    }
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite metadata error: {error}"),
            Self::Naming(error) => write!(f, "{error}"),
            Self::ActionSchema(error) => write!(f, "Action schema decode error: {error}"),
            Self::ActionSchemaNotFound(action_schema_id) => {
                write!(f, "Missing action schema: {action_schema_id}")
            }
            Self::AbstractLineNotFound {
                strategy,
                player_count,
                depth_bb,
                abstract_line,
            } => write!(
                f,
                "No concrete lines found for abstract_line={abstract_line} in dimension {strategy}:{player_count}:{depth_bb}"
            ),
            Self::ConcreteLineValueNotFound {
                strategy,
                player_count,
                depth_bb,
                concrete_line,
            } => write!(
                f,
                "Concrete line not found: concrete_line={concrete_line}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
            Self::ConcreteLineFilterNotFound {
                strategy,
                player_count,
                depth_bb,
                abstract_line,
                concrete_line,
            } => write!(
                f,
                "Concrete line not found: abstract_line={abstract_line}, concrete_line={concrete_line}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
            Self::DrillScenarioNotFound {
                strategy,
                drill_name,
                player_count,
                drill_depth,
            } => write!(
                f,
                "No abstract lines found for drill: strategy={strategy}, drill_name={drill_name}, player_count={player_count}, drill_depth={drill_depth}"
            ),
        }
    }
}

impl std::error::Error for MetadataError {}

impl From<SqliteError> for MetadataError {
    fn from(error: SqliteError) -> Self {
        Self::Sqlite(error)
    }
}

impl From<NamingError> for MetadataError {
    fn from(error: NamingError) -> Self {
        Self::Naming(error)
    }
}

impl From<ActionSchemaError> for MetadataError {
    fn from(error: ActionSchemaError) -> Self {
        Self::ActionSchema(error)
    }
}

fn concrete_line_filter_not_found(
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    filter: ConcreteLineFilter<'_>,
) -> MetadataError {
    match filter {
        ConcreteLineFilter::Abstract(abstract_line) => MetadataError::AbstractLineNotFound {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
            abstract_line: abstract_line.to_owned(),
        },
        ConcreteLineFilter::Concrete(concrete_line) => MetadataError::ConcreteLineValueNotFound {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
            concrete_line: concrete_line.to_owned(),
        },
        ConcreteLineFilter::AbstractAndConcrete {
            abstract_line,
            concrete_line,
        } => MetadataError::ConcreteLineFilterNotFound {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
            abstract_line: abstract_line.to_owned(),
            concrete_line: concrete_line.to_owned(),
        },
    }
}
