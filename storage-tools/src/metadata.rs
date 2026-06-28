use std::collections::HashMap;
use std::path::Path;

use range_store_core::action_schema::{decode_action_blob, ActionDef};
use range_store_core::sqlite::Connection;

use crate::errors::ToolError;

/// Load all action schemas from a `meta.db` file.
pub fn load_action_schemas(meta_db_path: &Path) -> Result<HashMap<u32, Vec<ActionDef>>, ToolError> {
    let connection = Connection::open(meta_db_path, true)?;
    let mut statement = connection
        .prepare("SELECT id, action_count, action_blob FROM action_schemas ORDER BY id")?;
    statement.start(&[])?;
    let mut schemas = HashMap::new();
    while statement.step_row()? {
        let id = statement.column_u32(0)?;
        let action_count = statement.column_u32(1)?;
        let action_blob = statement.column_blob(2);
        let actions = decode_action_blob(&action_blob, action_count)
            .map_err(|error| ToolError::new("META_DB_ERROR", error.to_string()))?;
        schemas.insert(id, actions);
    }
    Ok(schemas)
}
