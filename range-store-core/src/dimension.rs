#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    InvalidDimensionSpec(String),
}

impl std::fmt::Display for NamingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsafeSqliteIdentifier(identifier) => {
                write!(f, "Unsafe SQLite identifier: {identifier}")
            }
            Self::InvalidDimensionSpec(spec) => {
                write!(
                    f,
                    "Invalid dimension '{spec}', expected strategy:player_count:depth_bb"
                )
            }
        }
    }
}

impl std::error::Error for NamingError {}

/// A dimension specification used during build and verification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DimensionSpec {
    pub strategy: String,
    pub player_count: u32,
    pub depth_bb: u32,
}

impl DimensionSpec {
    pub fn parse(value: &str) -> Result<Self, NamingError> {
        let parts: Vec<_> = value.split(':').collect();
        if parts.len() != 3 || parts[0].is_empty() {
            return Err(NamingError::InvalidDimensionSpec(value.to_owned()));
        }
        Ok(Self {
            strategy: parts[0].to_owned(),
            player_count: parts[1]
                .parse()
                .map_err(|_| NamingError::InvalidDimensionSpec(value.to_owned()))?,
            depth_bb: parts[2]
                .parse()
                .map_err(|_| NamingError::InvalidDimensionSpec(value.to_owned()))?,
        })
    }

    pub fn range_table(&self) -> String {
        format!(
            "range_data_{}_{}max_{}BB",
            self.strategy, self.player_count, self.depth_bb
        )
    }

    pub fn concrete_table(&self) -> String {
        get_concrete_lines_table_name(&self.strategy, self.player_count, self.depth_bb)
    }
}

/// Discover dimension tables from a source SQLite database.
pub fn discover_dimensions(
    connection: &crate::sqlite::Connection,
) -> Result<Vec<DimensionSpec>, crate::sqlite::SqliteError> {
    use crate::sqlite::Value;
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name LIKE 'range_data_%'
         ORDER BY name",
    )?;
    statement.start(&[])?;
    let mut dimensions = Vec::new();
    while statement.step_row()? {
        let name = statement.column_text(0)?;
        if let Some(dimension) = parse_range_table_name(&name) {
            let mut exists_statement = connection.prepare(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1
                 )",
            )?;
            exists_statement.start(&[Value::from(dimension.concrete_table())])?;
            let concrete_exists =
                exists_statement.step_row()? && exists_statement.column_i64(0) != 0;
            if concrete_exists {
                dimensions.push(dimension);
            }
        }
    }
    dimensions.sort_by(|left, right| {
        left.strategy
            .cmp(&right.strategy)
            .then(left.player_count.cmp(&right.player_count))
            .then(left.depth_bb.cmp(&right.depth_bb))
    });
    Ok(dimensions)
}

fn parse_range_table_name(name: &str) -> Option<DimensionSpec> {
    let suffix = name.strip_prefix("range_data_")?;
    let (strategy_and_players, depth_text) = suffix.rsplit_once("max_")?;
    let depth_bb = depth_text.strip_suffix("BB")?.parse().ok()?;
    let (strategy, player_count_text) = strategy_and_players.rsplit_once('_')?;
    let player_count = player_count_text.parse().ok()?;
    if strategy.is_empty() {
        return None;
    }
    Some(DimensionSpec {
        strategy: strategy.to_owned(),
        player_count,
        depth_bb,
    })
}
