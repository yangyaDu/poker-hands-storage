use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::Serialize;

use crate::action_schema::{decode_action_blob, ActionDef, ActionSchemaError};
use crate::dimension::{
    get_concrete_lines_table_name, get_drill_scenario_table_name, quote_identifier, NamingError,
};
use crate::manifest::{load_manifest, queryable_dimensions};
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

/// A lazily populated in-memory metadata index.
///
/// Construction only reads the manifest. Concrete-line and drill metadata are
/// loaded into HashMaps on first access for the requested dimension/strategy.
#[derive(Debug)]
pub struct CachedMetadataReader {
    meta_path: PathBuf,
    /// strategy -> list of dimensions under that strategy
    strategies: Vec<String>,
    dimensions: HashSet<ConcreteDimensionKey>,
    state: RwLock<CachedMetadataState>,
}

#[derive(Debug, Default)]
struct CachedMetadataState {
    /// (strategy, player_count, depth_bb, concrete_line) -> ConcreteLineRow
    concrete_by_concrete: HashMap<ConcreteByConcreteKey, ConcreteLineRow>,
    /// (strategy, player_count, depth_bb, abstract_line) -> Vec<ConcreteLineRow>
    concrete_by_abstract: HashMap<ConcreteByAbstractKey, Vec<ConcreteLineRow>>,
    /// (strategy, drill_name, player_count, drill_depth) -> Vec<String>
    drill_lines: HashMap<DrillKey, Vec<String>>,
    loaded_concrete_dimensions: HashSet<ConcreteDimensionKey>,
    loaded_drill_strategies: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConcreteDimensionKey {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConcreteByConcreteKey {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    concrete_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConcreteByAbstractKey {
    strategy: String,
    player_count: u32,
    depth_bb: u32,
    abstract_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DrillKey {
    strategy: String,
    drill_name: String,
    player_count: u32,
    drill_depth: u32,
}

impl CachedMetadataReader {
    pub fn load(data_dir: &Path, meta_path: &Path) -> Result<Self, MetadataError> {
        // Discover strategies from manifest
        let manifest_path = data_dir.join("manifest.json");
        let manifest = load_manifest(&manifest_path).map_err(|e| {
            MetadataError::Sqlite(SqliteError(format!("Failed to load manifest: {e}")))
        })?;
        let dimensions = queryable_dimensions(&manifest).map_err(|e| {
            MetadataError::Sqlite(SqliteError(format!("Failed to parse manifest: {e}")))
        })?;

        // Collect unique strategy names
        let mut strategies: Vec<String> = dimensions
            .iter()
            .map(|d| d.strategy.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        strategies.sort();

        let dimensions = dimensions
            .into_iter()
            .map(|dimension| ConcreteDimensionKey {
                strategy: dimension.strategy,
                player_count: dimension.player_count,
                depth_bb: dimension.depth_bb,
            })
            .collect();

        Ok(Self {
            meta_path: meta_path.to_path_buf(),
            strategies,
            dimensions,
            state: RwLock::new(CachedMetadataState::default()),
        })
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, CachedMetadataState>, MetadataError> {
        self.state.read().map_err(|_| metadata_cache_lock_error())
    }

    fn write_state(&self) -> Result<RwLockWriteGuard<'_, CachedMetadataState>, MetadataError> {
        self.state.write().map_err(|_| metadata_cache_lock_error())
    }

    fn ensure_concrete_dimension_loaded(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Result<(), MetadataError> {
        let dimension = ConcreteDimensionKey {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
        };
        if !self.dimensions.contains(&dimension) {
            return Ok(());
        }

        {
            let state = self.read_state()?;
            if state.loaded_concrete_dimensions.contains(&dimension) {
                return Ok(());
            }
        }

        let mut state = self.write_state()?;
        if state.loaded_concrete_dimensions.contains(&dimension) {
            return Ok(());
        }
        let connection = Connection::open(&self.meta_path, true)?;
        load_concrete_dimension_for_dim(&mut state, &connection, strategy, player_count, depth_bb)?;
        state.loaded_concrete_dimensions.insert(dimension);
        Ok(())
    }

    fn ensure_drill_strategy_loaded(&self, strategy: &str) -> Result<(), MetadataError> {
        if !self.strategies.iter().any(|known| known == strategy) {
            return Ok(());
        }

        {
            let state = self.read_state()?;
            if state.loaded_drill_strategies.contains(strategy) {
                return Ok(());
            }
        }

        let mut state = self.write_state()?;
        if state.loaded_drill_strategies.contains(strategy) {
            return Ok(());
        }
        let connection = Connection::open(&self.meta_path, true)?;
        load_drill_strategy(&mut state, &connection, strategy)?;
        state.loaded_drill_strategies.insert(strategy.to_owned());
        Ok(())
    }

    /// Look up the concrete_line_id for a specific (dimension, concrete_line).
    pub fn get_concrete_line_id(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        concrete_line: &str,
    ) -> Result<Option<u32>, MetadataError> {
        self.ensure_concrete_dimension_loaded(strategy, player_count, depth_bb)?;
        let state = self.read_state()?;
        Ok(state
            .concrete_by_concrete
            .get(&ConcreteByConcreteKey {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                concrete_line: concrete_line.to_owned(),
            })
            .map(|row| row.concrete_line_id))
    }

    /// Get concrete lines filtered by abstract_line and/or concrete_line.
    pub fn get_concrete_lines(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        abstract_line: Option<&str>,
        concrete_line: Option<&str>,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError> {
        self.ensure_concrete_dimension_loaded(strategy, player_count, depth_bb)?;
        let state = self.read_state()?;
        match (
            abstract_line.map(|v| v.trim()),
            concrete_line.map(|v| v.trim()),
        ) {
            (Some(abstract_), Some(conc)) if !abstract_.is_empty() && !conc.is_empty() => {
                let key = ConcreteByConcreteKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    concrete_line: conc.to_owned(),
                };
                match state.concrete_by_concrete.get(&key) {
                    Some(row) if row.abstract_line == abstract_ => Ok(vec![row.clone()]),
                    Some(_) => Err(MetadataError::ConcreteLineFilterNotFound {
                        strategy: strategy.to_owned(),
                        player_count,
                        depth_bb,
                        abstract_line: abstract_.to_owned(),
                        concrete_line: conc.to_owned(),
                    }),
                    None => Err(MetadataError::ConcreteLineFilterNotFound {
                        strategy: strategy.to_owned(),
                        player_count,
                        depth_bb,
                        abstract_line: abstract_.to_owned(),
                        concrete_line: conc.to_owned(),
                    }),
                }
            }
            (Some(abstract_), None) if !abstract_.is_empty() => {
                let key = ConcreteByAbstractKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    abstract_line: abstract_.to_owned(),
                };
                state
                    .concrete_by_abstract
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| MetadataError::AbstractLineNotFound {
                        strategy: strategy.to_owned(),
                        player_count,
                        depth_bb,
                        abstract_line: abstract_.to_owned(),
                    })
            }
            (None, Some(conc)) if !conc.is_empty() => {
                let key = ConcreteByConcreteKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    concrete_line: conc.to_owned(),
                };
                state
                    .concrete_by_concrete
                    .get(&key)
                    .cloned()
                    .map(|row| vec![row])
                    .ok_or_else(|| MetadataError::ConcreteLineValueNotFound {
                        strategy: strategy.to_owned(),
                        player_count,
                        depth_bb,
                        concrete_line: conc.to_owned(),
                    })
            }
            _ => Err(MetadataError::AbstractLineNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: String::new(),
            }),
        }
    }

    /// Get drill scenario abstract lines.
    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, MetadataError> {
        self.ensure_drill_strategy_loaded(strategy)?;
        let state = self.read_state()?;
        state
            .drill_lines
            .get(&DrillKey {
                strategy: strategy.to_owned(),
                drill_name: drill_name.to_owned(),
                player_count,
                drill_depth,
            })
            .cloned()
            .ok_or_else(|| MetadataError::DrillScenarioNotFound {
                strategy: strategy.to_owned(),
                drill_name: drill_name.to_owned(),
                player_count,
                drill_depth,
            })
    }

    /// Return the list of known strategy names.
    pub fn strategies(&self) -> &[String] {
        &self.strategies
    }
}

fn load_concrete_dimension_for_dim(
    state: &mut CachedMetadataState,
    connection: &Connection,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
) -> Result<(), MetadataError> {
    let table = quote_identifier(&get_concrete_lines_table_name(
        strategy,
        player_count,
        depth_bb,
    ))?;
    let sql = format!(
        "SELECT concrete_line_id, abstract_line, concrete_line \
             FROM {table} \
             ORDER BY concrete_line_id"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[])?;

    while statement.step_row()? {
        let row = ConcreteLineRow {
            concrete_line_id: statement.column_u32(0)?,
            abstract_line: statement.column_text(1)?,
            concrete_line: statement.column_text(2)?,
        };
        let concrete_key = ConcreteByConcreteKey {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
            concrete_line: row.concrete_line.clone(),
        };
        state.concrete_by_concrete.insert(concrete_key, row.clone());

        let abstract_key = ConcreteByAbstractKey {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
            abstract_line: row.abstract_line.clone(),
        };
        state
            .concrete_by_abstract
            .entry(abstract_key)
            .or_default()
            .push(row);
    }
    Ok(())
}

fn load_drill_strategy(
    state: &mut CachedMetadataState,
    connection: &Connection,
    strategy: &str,
) -> Result<(), MetadataError> {
    let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
    let sql = format!(
        "SELECT drill_name, abstract_line, player_count, drill_depth \
             FROM {table} \
             ORDER BY drill_name, player_count, drill_depth, abstract_line"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[])?;

    while statement.step_row()? {
        let key = DrillKey {
            strategy: strategy.to_owned(),
            drill_name: statement.column_text(0)?,
            player_count: statement.column_u32(2)?,
            drill_depth: statement.column_u32(3)?,
        };
        state
            .drill_lines
            .entry(key)
            .or_default()
            .push(statement.column_text(1)?);
    }
    Ok(())
}

fn metadata_cache_lock_error() -> MetadataError {
    MetadataError::Sqlite(SqliteError("metadata cache lock poisoned".to_owned()))
}
