use std::fmt;
use std::io;

use range_store_core::action_schema::{ActionSchemaError, ActionSchemaLoadError};
use range_store_core::dimension::NamingError;
use range_store_core::hole_cards::HandDictError;
use range_store_core::manifest::ManifestError;
use range_store_core::metadata::MetadataError;
use range_store_core::query::RangeStoreError;
use range_store_core::sqlite::SqliteError;

#[derive(Debug)]
pub struct AppError {
    code: &'static str,
    message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn public_code(&self) -> i32 {
        match self.code {
            "INVALID_ARGUMENT" => 1000,
            "BIN_FILE_NOT_FOUND"
            | "PACK_NOT_FOUND"
            | "DATA_FILE_NOT_FOUND"
            | "DRILL_SCENARIO_NOT_FOUND"
            | "ABSTRACT_LINE_NOT_FOUND"
            | "DIMENSION_NOT_FOUND"
            | "ACTION_SCHEMA_NOT_FOUND"
            | "CONCRETE_LINE_NOT_FOUND"
            | "HAND_STRATEGY_NOT_FOUND"
            | "ACTION_NOT_FOUND"
            | "HANDS_NOT_FOUND" => 404,
            "SERVICE_UNAVAILABLE" => 503,
            _ => 500,
        }
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }

    pub fn invalid_format(message: impl Into<String>) -> Self {
        Self::new("INVALID_FORMAT", message)
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self::new("BUILD_ERROR", message)
    }

    pub fn bin_file_not_found(message: impl Into<String>) -> Self {
        Self::new("BIN_FILE_NOT_FOUND", message)
    }

    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new("SERVICE_UNAVAILABLE", message)
    }

    pub fn dimension_not_found(strategy: &str, player_count: u32, depth_bb: u32) -> Self {
        Self::new(
            "DIMENSION_NOT_FOUND",
            format!(
                "Dimension not found: strategy={strategy}, player_count={player_count}, depth_bb={depth_bb}"
            ),
        )
    }

    pub fn concrete_line_not_found(
        concrete_line_id: u32,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Self {
        Self::new(
            "CONCRETE_LINE_NOT_FOUND",
            format!(
                "Concrete line not found: concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn concrete_line_value_not_found(
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        concrete_line: &str,
    ) -> Self {
        Self::new(
            "CONCRETE_LINE_NOT_FOUND",
            format!(
                "Concrete line not found: concrete_line={concrete_line}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn concrete_line_filter_not_found(
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        abstract_line: &str,
        concrete_line: &str,
    ) -> Self {
        Self::new(
            "CONCRETE_LINE_NOT_FOUND",
            format!(
                "Concrete line not found: abstract_line={abstract_line}, concrete_line={concrete_line}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn hand_outside_action_line(
        hole_cards: &str,
        concrete_line_id: u32,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Self {
        Self::new(
            "HAND_STRATEGY_NOT_FOUND",
            format!(
                "Hand {hole_cards} is outside the range for action line concrete_line_id={concrete_line_id} in dimension {strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn no_hands_found(
        actions: &str,
        frequency: &str,
        concrete_line_id: u32,
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
    ) -> Self {
        Self::new(
            "HANDS_NOT_FOUND",
            format!(
                "No hands found for actions={actions} at frequency{frequency}, concrete_line_id={concrete_line_id}, dimension={strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn drill_scenario_not_found(
        strategy: &str,
        drill_name: &str,
        player_count: u32,
        drill_depth: u32,
    ) -> Self {
        Self::new(
            "DRILL_SCENARIO_NOT_FOUND",
            format!(
                "No abstract lines found for drill: strategy={strategy}, drill_name={drill_name}, player_count={player_count}, drill_depth={drill_depth}"
            ),
        )
    }

    pub fn abstract_line_not_found(
        strategy: &str,
        player_count: u32,
        depth_bb: u32,
        abstract_line: &str,
    ) -> Self {
        Self::new(
            "ABSTRACT_LINE_NOT_FOUND",
            format!(
                "No concrete lines found for abstract_line={abstract_line} in dimension {strategy}:{player_count}:{depth_bb}"
            ),
        )
    }

    pub fn action_schema_not_found(action_schema_id: u32) -> Self {
        Self::new(
            "ACTION_SCHEMA_NOT_FOUND",
            format!("Missing action schema: {action_schema_id}"),
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        let code = if error.kind() == io::ErrorKind::NotFound {
            "BIN_FILE_NOT_FOUND"
        } else {
            "INVALID_FORMAT"
        };
        Self::new(code, error.to_string())
    }
}

impl From<SqliteError> for AppError {
    fn from(error: SqliteError) -> Self {
        Self::new("META_DB_ERROR", error.to_string())
    }
}

impl From<ManifestError> for AppError {
    fn from(error: ManifestError) -> Self {
        Self::new("INVALID_FORMAT", error.to_string())
    }
}

impl From<ActionSchemaError> for AppError {
    fn from(error: ActionSchemaError) -> Self {
        Self::invalid_format(error.to_string())
    }
}

impl From<ActionSchemaLoadError> for AppError {
    fn from(error: ActionSchemaLoadError) -> Self {
        match error {
            ActionSchemaLoadError::Sqlite(error) => Self::new("META_DB_ERROR", error.to_string()),
            ActionSchemaLoadError::Schema(error) => Self::invalid_format(error.to_string()),
        }
    }
}

impl From<HandDictError> for AppError {
    fn from(error: HandDictError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}

impl From<NamingError> for AppError {
    fn from(error: NamingError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}

impl From<MetadataError> for AppError {
    fn from(error: MetadataError) -> Self {
        match error {
            MetadataError::Sqlite(error) => Self::new("META_DB_ERROR", error.to_string()),
            MetadataError::Naming(error) => Self::invalid_argument(error.to_string()),
            MetadataError::ActionSchema(error) => Self::invalid_format(error.to_string()),
            MetadataError::AbstractLineNotFound {
                strategy,
                player_count,
                depth_bb,
                abstract_line,
            } => Self::abstract_line_not_found(&strategy, player_count, depth_bb, &abstract_line),
            MetadataError::ConcreteLineValueNotFound {
                strategy,
                player_count,
                depth_bb,
                concrete_line,
            } => Self::concrete_line_value_not_found(
                &strategy,
                player_count,
                depth_bb,
                &concrete_line,
            ),
            MetadataError::ConcreteLineFilterNotFound {
                strategy,
                player_count,
                depth_bb,
                abstract_line,
                concrete_line,
            } => Self::concrete_line_filter_not_found(
                &strategy,
                player_count,
                depth_bb,
                &abstract_line,
                &concrete_line,
            ),
            MetadataError::DrillScenarioNotFound {
                strategy,
                drill_name,
                player_count,
                drill_depth,
            } => Self::drill_scenario_not_found(&strategy, &drill_name, player_count, drill_depth),
        }
    }
}

impl From<RangeStoreError> for AppError {
    fn from(error: RangeStoreError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}
