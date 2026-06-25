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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_current_file_naming() {
        assert_eq!(
            get_idx_file_name("default", 6, 100),
            "ranges_default_6max_100BB.idx"
        );
        assert_eq!(
            get_bin_file_name("default", 6, 100),
            "ranges_default_6max_100BB.bin"
        );
    }

    #[test]
    fn matches_current_table_naming() {
        assert_eq!(
            get_drill_scenario_table_name("default"),
            "drill_scenario_lines_default"
        );
        assert_eq!(
            get_concrete_lines_table_name("default", 9, 300),
            "concrete_lines_default_9max_300BB"
        );
    }

    #[test]
    fn quote_identifier_matches_typescript_guardrail() {
        assert_eq!(
            quote_identifier("concrete_lines_default_6max_100BB").unwrap(),
            "\"concrete_lines_default_6max_100BB\""
        );
        assert!(quote_identifier("../escape").is_err());
        assert!(quote_identifier("9bad").is_err());
    }
}
