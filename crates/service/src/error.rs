use std::fmt;
use std::io;

use crate::action_schema::ActionSchemaError;
use crate::hand_dict::HandDictError;
use crate::manifest::ManifestError;
use crate::naming::NamingError;

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

impl From<crate::sqlite::SqliteError> for AppError {
    fn from(error: crate::sqlite::SqliteError) -> Self {
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

impl From<HandDictError> for AppError {
    fn from(error: HandDictError) -> Self {
        Self::new("UNKNOWN_HAND", error.to_string())
    }
}

impl From<NamingError> for AppError {
    fn from(error: NamingError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}
