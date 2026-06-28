use std::collections::HashMap;
use std::path::Path;

use range_store_core::action_schema::ActionDef;

use crate::errors::ToolError;

/// Load all action schemas from a `meta.db` file.
pub fn load_action_schemas(meta_db_path: &Path) -> Result<HashMap<u32, Vec<ActionDef>>, ToolError> {
    range_store_core::action_schema::load_action_schemas(meta_db_path)
        .map_err(|error| ToolError::new("META_DB_ERROR", error.to_string()))
}
