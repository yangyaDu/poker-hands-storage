use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use utoipa::ToSchema;

use crate::action_schema::{decode_action_blob, ActionDef};
use crate::error::AppError;
use crate::naming::{
    get_concrete_lines_table_name, get_drill_scenario_table_name, quote_identifier,
};
use crate::sqlite::{Connection, Value};

#[derive(Debug, Clone)]
pub struct MetadataReader {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, ToSchema, PartialEq, Eq)]
pub struct ConcreteLineRow {
    /// Concrete line id used by range queries.
    pub concrete_line_id: u32,
    /// Abstract action line.
    pub abstract_line: String,
    /// Concrete action line.
    pub concrete_line: String,
}

impl MetadataReader {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_action_schemas(&self) -> Result<HashMap<u32, Vec<ActionDef>>, AppError> {
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
    ) -> Result<(), AppError> {
        let connection = self.open()?;
        let mut statement =
            connection.prepare("SELECT DISTINCT action_schema_id FROM dimension_action_schemas")?;
        statement.start(&[])?;
        while statement.step_row()? {
            let action_schema_id = statement.column_u32(0)?;
            if !action_schema_ids.contains(&action_schema_id) {
                return Err(AppError::action_schema_not_found(action_schema_id));
            }
        }
        Ok(())
    }

    pub fn dimension_action_schema_ids(
        &self,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Result<Vec<u32>, AppError> {
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
        abstract_line: &str,
    ) -> Result<Vec<ConcreteLineRow>, AppError> {
        let table = quote_identifier(&get_concrete_lines_table_name(
            strategy,
            player_count,
            depth_bb,
        ))?;
        let connection = self.open()?;
        let sql = format!(
            "SELECT concrete_line_id, abstract_line, concrete_line
             FROM {table}
             WHERE abstract_line = ?1
             ORDER BY concrete_line_id"
        );
        let mut statement = connection.prepare(&sql)?;
        statement.start(&[Value::from(abstract_line)])?;
        let mut lines = Vec::new();
        while statement.step_row()? {
            lines.push(ConcreteLineRow {
                concrete_line_id: statement.column_u32(0)?,
                abstract_line: statement.column_text(1)?,
                concrete_line: statement.column_text(2)?,
            });
        }
        Ok(lines)
    }

    pub fn get_drill_scenario_lines(
        &self,
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Result<Vec<String>, AppError> {
        let table = quote_identifier(&get_drill_scenario_table_name(strategy))?;
        let connection = self.open()?;
        let sql = format!(
            "SELECT abstract_line
             FROM {table}
             WHERE drill_name = ?1 AND player_count = ?2 AND drill_depth = ?3
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

    fn open(&self) -> Result<Connection, AppError> {
        Connection::open(&self.path, true).map_err(AppError::from)
    }
}
