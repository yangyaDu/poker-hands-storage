#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionName {
    Fold,
    Call,
    Check,
    Bet,
    Raise,
    Allin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionDef {
    pub action_id: u32,
    pub action_name: ActionName,
    pub action_size: f32,
    pub amount_bb: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionSchemaError {
    InvalidLength { expected: usize, got: usize },
    InvalidActionCount(u32),
    UnknownActionType(u8),
}

pub fn decode_action_blob(
    blob: &[u8],
    action_count: u32,
) -> Result<Vec<ActionDef>, ActionSchemaError> {
    if !(1..=32).contains(&action_count) {
        return Err(ActionSchemaError::InvalidActionCount(action_count));
    }

    let expected = action_count as usize * 9;
    if blob.len() != expected {
        return Err(ActionSchemaError::InvalidLength {
            expected,
            got: blob.len(),
        });
    }

    let mut actions = Vec::with_capacity(action_count as usize);
    let mut cursor = 0usize;
    for action_id in 0..action_count {
        let action_type = blob[cursor];
        cursor += 1;
        let action_size = f32::from_le_bytes([
            blob[cursor],
            blob[cursor + 1],
            blob[cursor + 2],
            blob[cursor + 3],
        ]);
        cursor += 4;
        let amount_bb = f32::from_le_bytes([
            blob[cursor],
            blob[cursor + 1],
            blob[cursor + 2],
            blob[cursor + 3],
        ]);
        cursor += 4;

        let action_name = action_name_by_type(action_type)
            .ok_or(ActionSchemaError::UnknownActionType(action_type))?;
        actions.push(ActionDef {
            action_id,
            action_name,
            action_size,
            amount_bb,
        });
    }

    Ok(actions)
}

fn action_name_by_type(action_type: u8) -> Option<ActionName> {
    match action_type {
        0 => Some(ActionName::Fold),
        1 => Some(ActionName::Call),
        2 => Some(ActionName::Check),
        3 => Some(ActionName::Bet),
        4 => Some(ActionName::Raise),
        5 => Some(ActionName::Allin),
        _ => None,
    }
}

impl ActionName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fold => "fold",
            Self::Call => "call",
            Self::Check => "check",
            Self::Bet => "bet",
            Self::Raise => "raise",
            Self::Allin => "allin",
        }
    }
}

impl std::fmt::Display for ActionSchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength { expected, got } => {
                write!(
                    f,
                    "Invalid action schema length: expected {expected}, got {got}"
                )
            }
            Self::InvalidActionCount(count) => {
                write!(f, "Invalid action count: {count}, expected 1..=32")
            }
            Self::UnknownActionType(action_type) => {
                write!(f, "Unknown action type: {action_type}")
            }
        }
    }
}

impl std::error::Error for ActionSchemaError {}

use std::collections::HashMap;
use std::path::Path;

use crate::sqlite::{Connection, SqliteError};

/// Load all action schemas from a `meta.db` SQLite file.
pub fn load_action_schemas(
    meta_db_path: &Path,
) -> Result<HashMap<u32, Vec<ActionDef>>, ActionSchemaLoadError> {
    let connection = Connection::open(meta_db_path, true)?;
    let mut statement = connection
        .prepare("SELECT id, action_count, action_blob FROM action_schemas ORDER BY id")?;
    statement.start(&[])?;
    let mut schemas = HashMap::new();
    while statement.step_row()? {
        let id = statement.column_u32(0)?;
        let action_count = statement.column_u32(1)?;
        let action_blob = statement.column_blob(2);
        let actions = decode_action_blob(&action_blob, action_count)?;
        schemas.insert(id, actions);
    }
    Ok(schemas)
}

/// Error returned by [`load_action_schemas`].
#[derive(Debug)]
pub enum ActionSchemaLoadError {
    Sqlite(SqliteError),
    Schema(ActionSchemaError),
}

impl std::fmt::Display for ActionSchemaLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "SQLite error loading action schemas: {e}"),
            Self::Schema(e) => write!(f, "Action schema decode error: {e}"),
        }
    }
}

impl std::error::Error for ActionSchemaLoadError {}

impl From<SqliteError> for ActionSchemaLoadError {
    fn from(error: SqliteError) -> Self {
        Self::Sqlite(error)
    }
}

impl From<ActionSchemaError> for ActionSchemaLoadError {
    fn from(error: ActionSchemaError) -> Self {
        Self::Schema(error)
    }
}
