use std::fmt;
use std::io;

use range_store_core::dimension::NamingError;
use range_store_core::hole_cards::HandDictError;
use range_store_core::manifest::ManifestError;
use range_store_core::sqlite::SqliteError;

#[derive(Debug)]
pub struct ToolError {
    code: &'static str,
    message: String,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn build(message: impl Into<String>) -> Self {
        Self::new("BUILD_ERROR", message)
    }

    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::new("INVALID_ARGUMENT", message)
    }

    pub fn invalid_format(message: impl Into<String>) -> Self {
        Self::new("INVALID_FORMAT", message)
    }

    pub fn verify(message: impl Into<String>) -> Self {
        Self::new("VERIFY_ERROR", message)
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

impl From<io::Error> for ToolError {
    fn from(error: io::Error) -> Self {
        Self::new("IO_ERROR", error.to_string())
    }
}

impl From<SqliteError> for ToolError {
    fn from(error: SqliteError) -> Self {
        Self::new("META_DB_ERROR", error.to_string())
    }
}

impl From<HandDictError> for ToolError {
    fn from(error: HandDictError) -> Self {
        Self::new("UNKNOWN_HAND", error.to_string())
    }
}

impl From<NamingError> for ToolError {
    fn from(error: NamingError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}

impl From<ManifestError> for ToolError {
    fn from(error: ManifestError) -> Self {
        Self::new("MANIFEST_ERROR", error.to_string())
    }
}
