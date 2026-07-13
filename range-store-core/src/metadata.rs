use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

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
/// Construction only reads the manifest. Metadata rows are queried from
/// `meta.db` on first access for the requested key, then cached in memory.
#[derive(Debug)]
pub struct CachedMetadataReader {
    connection: Mutex<LockedMetadataConnection>,
    /// strategy -> list of dimensions under that strategy
    strategies: Vec<String>,
    dimensions: HashSet<ConcreteDimensionKey>,
    state: RwLock<CachedMetadataState>,
}

struct LockedMetadataConnection {
    connection: Connection,
}

impl std::fmt::Debug for LockedMetadataConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockedMetadataConnection")
            .finish_non_exhaustive()
    }
}

// The SQLite wrapper opens read-only metadata handles with SQLITE_OPEN_NOMUTEX.
// This private wrapper is only exposed behind CachedMetadataReader's Mutex, and
// statements are prepared/stepped/finalized while the mutex guard is held.
unsafe impl Send for LockedMetadataConnection {}

#[derive(Debug, Default)]
struct CachedMetadataState {
    /// (strategy, player_count, depth_bb, concrete_line) -> ConcreteLineRow
    concrete_by_concrete: HashMap<ConcreteByConcreteKey, ConcreteLineRow>,
    /// (strategy, player_count, depth_bb, abstract_line) -> Vec<ConcreteLineRow>
    concrete_by_abstract: HashMap<ConcreteByAbstractKey, Vec<ConcreteLineRow>>,
    /// (strategy, drill_name, player_count, drill_depth) -> Vec<String>
    drill_lines: HashMap<DrillKey, Vec<String>>,
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

        let connection = Connection::open(meta_path, true)?;

        Ok(Self {
            connection: Mutex::new(LockedMetadataConnection { connection }),
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

    fn connection(&self) -> Result<MutexGuard<'_, LockedMetadataConnection>, MetadataError> {
        self.connection
            .lock()
            .map_err(|_| metadata_cache_lock_error())
    }

    fn concrete_dimension_known(&self, strategy: &str, player_count: u32, depth_bb: u32) -> bool {
        self.dimensions.contains(&ConcreteDimensionKey {
            strategy: strategy.to_owned(),
            player_count,
            depth_bb,
        })
    }

    fn drill_strategy_known(&self, strategy: &str) -> bool {
        self.strategies.iter().any(|known| known == strategy)
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
        let abstract_line = abstract_line.map(str::trim);
        let concrete_line = concrete_line.map(str::trim);
        match (abstract_line, concrete_line) {
            (Some(abstract_), Some(conc)) if !abstract_.is_empty() && !conc.is_empty() => self
                .get_concrete_lines_by_abstract_and_concrete(
                    strategy,
                    player_count,
                    depth_bb,
                    abstract_,
                    conc,
                ),
            (Some(abstract_), None) if !abstract_.is_empty() => {
                self.get_concrete_lines_by_abstract(strategy, player_count, depth_bb, abstract_)
            }
            (None, Some(conc)) if !conc.is_empty() => {
                self.get_concrete_lines_by_concrete(strategy, player_count, depth_bb, conc)
            }
            _ => Err(MetadataError::AbstractLineNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: String::new(),
            }),
        }
    }

    fn read_through_cached_concrete<L, S>(
        &self,
        cache_lookup: impl Fn(
            &CachedMetadataState,
        ) -> Result<Option<Vec<ConcreteLineRow>>, MetadataError>,
        loader: L,
        on_empty: impl Fn() -> MetadataError,
        cache_store: S,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError>
    where
        L: FnOnce(&Connection) -> Result<Vec<ConcreteLineRow>, MetadataError>,
        S: FnOnce(&mut CachedMetadataState, &[ConcreteLineRow]),
    {
        {
            let state = self.read_state()?;
            if let Some(cached) = cache_lookup(&state)? {
                return Ok(cached);
            }
        }
        let connection = self.connection()?;
        let rows = loader(&connection.connection)?;
        drop(connection);
        if rows.is_empty() {
            return Err(on_empty());
        }
        let mut state = self.write_state()?;
        cache_store(&mut state, &rows);
        Ok(rows)
    }

    fn get_concrete_lines_by_abstract(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        abstract_line: &str,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError> {
        if !self.concrete_dimension_known(strategy, player_count, depth_bb) {
            return Err(MetadataError::AbstractLineNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: abstract_line.to_owned(),
            });
        }

        self.read_through_cached_concrete(
            |state| {
                let key = ConcreteByAbstractKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    abstract_line: abstract_line.to_owned(),
                };
                Ok(state.concrete_by_abstract.get(&key).cloned())
            },
            |connection| {
                query_concrete_by_abstract(
                    connection,
                    strategy,
                    player_count,
                    depth_bb,
                    abstract_line,
                )
            },
            || MetadataError::AbstractLineNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: abstract_line.to_owned(),
            },
            |state, rows| {
                let key = ConcreteByAbstractKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    abstract_line: abstract_line.to_owned(),
                };
                state.concrete_by_abstract.insert(key, rows.to_vec());
                for row in rows {
                    cache_concrete_row(state, strategy, player_count, depth_bb, row.clone());
                }
            },
        )
    }

    fn get_concrete_lines_by_concrete(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        concrete_line: &str,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError> {
        if !self.concrete_dimension_known(strategy, player_count, depth_bb) {
            return Err(MetadataError::ConcreteLineValueNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                concrete_line: concrete_line.to_owned(),
            });
        }

        self.read_through_cached_concrete(
            |state| {
                let key = ConcreteByConcreteKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    concrete_line: concrete_line.to_owned(),
                };
                Ok(state
                    .concrete_by_concrete
                    .get(&key)
                    .map(|row| vec![row.clone()]))
            },
            |connection| {
                query_concrete_by_concrete(
                    connection,
                    strategy,
                    player_count,
                    depth_bb,
                    concrete_line,
                )
            },
            || MetadataError::ConcreteLineValueNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                concrete_line: concrete_line.to_owned(),
            },
            |state, rows| {
                for row in rows {
                    cache_concrete_row(state, strategy, player_count, depth_bb, row.clone());
                }
            },
        )
    }

    fn get_concrete_lines_by_abstract_and_concrete(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        abstract_line: &str,
        concrete_line: &str,
    ) -> Result<Vec<ConcreteLineRow>, MetadataError> {
        if !self.concrete_dimension_known(strategy, player_count, depth_bb) {
            return Err(MetadataError::ConcreteLineFilterNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: abstract_line.to_owned(),
                concrete_line: concrete_line.to_owned(),
            });
        }

        self.read_through_cached_concrete(
            |state| {
                let key = ConcreteByConcreteKey {
                    strategy: strategy.to_owned(),
                    player_count,
                    depth_bb,
                    concrete_line: concrete_line.to_owned(),
                };
                if let Some(row) = state.concrete_by_concrete.get(&key) {
                    if row.abstract_line == abstract_line {
                        return Ok(Some(vec![row.clone()]));
                    }
                    return Err(MetadataError::ConcreteLineFilterNotFound {
                        strategy: strategy.to_owned(),
                        player_count,
                        depth_bb,
                        abstract_line: abstract_line.to_owned(),
                        concrete_line: concrete_line.to_owned(),
                    });
                }
                Ok(None)
            },
            |connection| {
                query_concrete_by_abstract_and_concrete(
                    connection,
                    strategy,
                    player_count,
                    depth_bb,
                    abstract_line,
                    concrete_line,
                )
            },
            || MetadataError::ConcreteLineFilterNotFound {
                strategy: strategy.to_owned(),
                player_count,
                depth_bb,
                abstract_line: abstract_line.to_owned(),
                concrete_line: concrete_line.to_owned(),
            },
            |state, rows| {
                for row in rows {
                    cache_concrete_row(state, strategy, player_count, depth_bb, row.clone());
                }
            },
        )
    }

    /// Get drill scenario abstract lines.
    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, MetadataError> {
        if !self.drill_strategy_known(strategy) {
            return Err(MetadataError::DrillScenarioNotFound {
                strategy: strategy.to_owned(),
                drill_name: drill_name.to_owned(),
                player_count,
                drill_depth,
            });
        }

        let key = DrillKey {
            strategy: strategy.to_owned(),
            drill_name: drill_name.to_owned(),
            player_count,
            drill_depth,
        };
        {
            let state = self.read_state()?;
            if let Some(lines) = state.drill_lines.get(&key) {
                return Ok(lines.clone());
            }
        }

        let connection = self.connection()?;
        let lines = query_drill_lines(
            &connection.connection,
            strategy,
            drill_name,
            player_count,
            drill_depth,
        )?;
        drop(connection);
        if lines.is_empty() {
            return Err(MetadataError::DrillScenarioNotFound {
                strategy: strategy.to_owned(),
                drill_name: drill_name.to_owned(),
                player_count,
                drill_depth,
            });
        }
        let mut state = self.write_state()?;
        state.drill_lines.insert(key, lines.clone());
        Ok(lines)
    }

    /// Return the list of known strategy names.
    pub fn strategies(&self) -> &[String] {
        &self.strategies
    }
}

fn cache_concrete_row(
    state: &mut CachedMetadataState,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    row: ConcreteLineRow,
) {
    let concrete_key = ConcreteByConcreteKey {
        strategy: strategy.to_owned(),
        player_count,
        depth_bb,
        concrete_line: row.concrete_line.clone(),
    };
    state.concrete_by_concrete.insert(concrete_key, row);
}

fn query_concrete_by_abstract(
    connection: &Connection,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    abstract_line: &str,
) -> Result<Vec<ConcreteLineRow>, MetadataError> {
    query_concrete_lines(
        connection,
        strategy,
        player_count,
        depth_bb,
        "abstract_line = ?1",
        vec![Value::from(abstract_line)],
    )
}

fn query_concrete_by_concrete(
    connection: &Connection,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    concrete_line: &str,
) -> Result<Vec<ConcreteLineRow>, MetadataError> {
    query_concrete_lines(
        connection,
        strategy,
        player_count,
        depth_bb,
        "concrete_line = ?1",
        vec![Value::from(concrete_line)],
    )
}

fn query_concrete_by_abstract_and_concrete(
    connection: &Connection,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    abstract_line: &str,
    concrete_line: &str,
) -> Result<Vec<ConcreteLineRow>, MetadataError> {
    query_concrete_lines(
        connection,
        strategy,
        player_count,
        depth_bb,
        "abstract_line = ?1 AND concrete_line = ?2",
        vec![Value::from(abstract_line), Value::from(concrete_line)],
    )
}

fn query_concrete_lines(
    connection: &Connection,
    strategy: &str,
    player_count: u32,
    depth_bb: u32,
    where_clause: &str,
    values: Vec<Value>,
) -> Result<Vec<ConcreteLineRow>, MetadataError> {
    let table = quote_identifier(&get_concrete_lines_table_name(
        strategy,
        player_count,
        depth_bb,
    ))?;
    let sql = format!(
        "SELECT concrete_line_id, abstract_line, concrete_line \
             FROM {table} \
             WHERE {where_clause} \
             ORDER BY concrete_line_id"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&values)?;

    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push(ConcreteLineRow {
            concrete_line_id: statement.column_u32(0)?,
            abstract_line: statement.column_text(1)?,
            concrete_line: statement.column_text(2)?,
        });
    }
    Ok(rows)
}

fn query_drill_lines(
    connection: &Connection,
    strategy: &str,
    drill_name: &str,
    player_count: u32,
    drill_depth: u32,
) -> Result<Vec<String>, MetadataError> {
    let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
    let sql = format!(
        "SELECT abstract_line \
             FROM {table} \
             WHERE drill_name = ?1 AND player_count = ?2 AND drill_depth = ?3 \
             ORDER BY abstract_line"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.start(&[
        Value::from(drill_name),
        Value::from(player_count),
        Value::from(drill_depth),
    ])?;

    let mut lines = Vec::new();
    while statement.step_row()? {
        lines.push(statement.column_text(0)?);
    }
    Ok(lines)
}

fn metadata_cache_lock_error() -> MetadataError {
    MetadataError::Sqlite(SqliteError("metadata cache lock poisoned".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn cached_metadata_reader_loads_only_requested_keys() {
        let temp = tempfile::TempDir::new().unwrap();
        let meta_path = build_metadata_fixture(temp.path());
        let reader = CachedMetadataReader::load(temp.path(), &meta_path).unwrap();

        {
            let state = reader.read_state().unwrap();
            assert_eq!(state.concrete_by_concrete.len(), 0);
            assert_eq!(state.concrete_by_abstract.len(), 0);
            assert!(state.drill_lines.is_empty());
        }

        let abstract_rows = reader
            .get_concrete_lines("default", 6, 100, Some("F-F-F"), None)
            .unwrap();
        assert_eq!(abstract_rows.len(), 2);

        {
            let state = reader.read_state().unwrap();
            assert_eq!(state.concrete_by_concrete.len(), 2);
            assert_eq!(state.concrete_by_abstract.len(), 1);
            assert!(state.drill_lines.is_empty());
        }

        let exact_rows = reader
            .get_concrete_lines("default", 6, 100, Some("F-F-F"), Some("F-F-F-R2"))
            .unwrap();
        assert_eq!(exact_rows.len(), 1);
        assert_eq!(exact_rows[0].concrete_line_id, 2);

        let drill_lines = reader
            .get_drill_scenario_lines("default", "rfi", 6, 100)
            .unwrap();
        assert_eq!(drill_lines, vec!["F-F-F".to_owned(), "F-F-F-R2".to_owned()]);
    }

    #[test]
    fn cached_metadata_reader_returns_not_found_for_unknown_dimension_without_sqlite_error() {
        let temp = tempfile::TempDir::new().unwrap();
        let meta_path = build_metadata_fixture(temp.path());
        let reader = CachedMetadataReader::load(temp.path(), &meta_path).unwrap();

        let error = reader
            .get_concrete_lines("default", 9, 100, Some("F-F-F"), None)
            .unwrap_err();
        assert!(matches!(error, MetadataError::AbstractLineNotFound { .. }));
    }

    fn build_metadata_fixture(root: &Path) -> PathBuf {
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "format": "PFSP",
                "version": 1,
                "sourceDbChecksum": "fixture",
                "builtAt": "2026-07-05T00:00:00Z",
                "dimensions": [{
                    "strategy": "default",
                    "playerCount": 6,
                    "depthBb": 100,
                    "concreteLineCount": 2,
                    "packCount": 2,
                    "status": "success",
                    "binFile": "ranges_default_6max_100BB.bin",
                    "idxFile": "ranges_default_6max_100BB.idx"
                }],
                "files": ["meta.db", "ranges_default_6max_100BB.bin", "ranges_default_6max_100BB.idx"]
            }))
            .unwrap(),
        )
        .unwrap();

        let meta_path = root.join("meta.db");
        let connection = Connection::open(&meta_path, false).unwrap();
        connection
            .exec(
                "CREATE TABLE concrete_lines_default_6max_100BB (
                   concrete_line_id INTEGER PRIMARY KEY,
                   abstract_line TEXT NOT NULL,
                   concrete_line TEXT NOT NULL,
                   UNIQUE(abstract_line, concrete_line)
                 );
                 CREATE INDEX idx_concrete_lines_default_6max_100BB_concrete_line
                   ON concrete_lines_default_6max_100BB(concrete_line);
                 CREATE TABLE drill_scenario_lines_default (
                   id INTEGER PRIMARY KEY AUTOINCREMENT,
                   drill_name TEXT NOT NULL,
                   abstract_line TEXT NOT NULL,
                   player_count INTEGER NOT NULL,
                   drill_depth INTEGER NOT NULL DEFAULT 100,
                   UNIQUE(drill_name, player_count, drill_depth, abstract_line)
                 );
                 INSERT INTO concrete_lines_default_6max_100BB
                   VALUES (1, 'F-F-F', 'F-F-F');
                 INSERT INTO concrete_lines_default_6max_100BB
                   VALUES (2, 'F-F-F', 'F-F-F-R2');
                 INSERT INTO drill_scenario_lines_default(
                   drill_name, abstract_line, player_count, drill_depth
                 ) VALUES ('rfi', 'F-F-F', 6, 100);
                 INSERT INTO drill_scenario_lines_default(
                   drill_name, abstract_line, player_count, drill_depth
                 ) VALUES ('rfi', 'F-F-F-R2', 6, 100);",
            )
            .unwrap();
        meta_path
    }
}
