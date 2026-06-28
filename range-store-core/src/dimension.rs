#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DimensionRef {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
}

impl DimensionRef {
    pub fn new(strategy: impl Into<String>, player_count: u32, depth_bb: u32) -> Self {
        Self {
            strategy: strategy.into(),
            player_count,
            depth_bb,
        }
    }

    pub fn with_default_strategy(player_count: u32, depth_bb: u32) -> Self {
        Self::new("default", player_count, depth_bb)
    }
}

pub fn dimension_key(dimension: &DimensionRef) -> String {
    format!(
        "{}:{}max:{}BB",
        dimension.strategy, dimension.player_count, dimension.depth_bb
    )
}

pub fn get_idx_file_name(strategy: &str, player_count: u32, depth_bb: u32) -> String {
    format!("ranges_{strategy}_{player_count}max_{depth_bb}BB.idx")
}

pub fn get_bin_file_name(strategy: &str, player_count: u32, depth_bb: u32) -> String {
    format!("ranges_{strategy}_{player_count}max_{depth_bb}BB.bin")
}

pub fn get_drill_scenario_table_name(strategy: &str) -> String {
    format!("drill_scenario_lines_{strategy}")
}

pub fn get_concrete_lines_table_name(strategy: &str, player_count: u32, depth_bb: u32) -> String {
    format!("concrete_lines_{strategy}_{player_count}max_{depth_bb}BB")
}

pub fn quote_identifier(identifier: &str) -> Result<String, NamingError> {
    if is_safe_sqlite_identifier(identifier) {
        Ok(format!("\"{identifier}\""))
    } else {
        Err(NamingError::UnsafeSqliteIdentifier(identifier.to_owned()))
    }
}

fn is_safe_sqlite_identifier(identifier: &str) -> bool {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamingError {
    UnsafeSqliteIdentifier(String),
}

impl std::fmt::Display for NamingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeSqliteIdentifier(identifier) => {
                write!(f, "Unsafe SQLite identifier: {identifier}")
            }
        }
    }
}

impl std::error::Error for NamingError {}
