use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::errors::AppError;
use crate::http::request_validation::ValidationErrorDetails;

/// Public API status codes. Detailed failure reasons belong in `message`.
pub mod error_code {
    pub const SUCCESS: i32 = 0;
    pub const BAD_REQUEST: i32 = 1000;
    pub const NOT_FOUND: i32 = 404;
    pub const INTERNAL_SERVER_ERROR: i32 = 500;
    pub const SERVICE_UNAVAILABLE: i32 = 503;
}

#[derive(Debug)]
pub struct HttpError {
    error: AppError,
}

impl HttpError {
    pub fn invalid_json(message: impl Into<String>) -> Self {
        Self {
            error: AppError::invalid_argument(message),
        }
    }

    pub fn validation(details: ValidationErrorDetails) -> Self {
        Self {
            error: AppError::invalid_argument(format!(
                "request validation failed: {}",
                details.message()
            )),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            error: AppError::invalid_argument(message),
        }
    }
}

impl From<AppError> for HttpError {
    fn from(error: AppError) -> Self {
        Self { error }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ErrorResponseBody {
    /// Business status code. Non-zero values indicate errors.
    code: i32,
    /// Error responses do not carry endpoint data.
    #[schema(nullable)]
    data: Option<()>,
    /// Human-readable message.
    message: String,
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let code = self.error.public_code();
        let status = match code {
            error_code::BAD_REQUEST => StatusCode::BAD_REQUEST,
            error_code::NOT_FOUND => StatusCode::NOT_FOUND,
            error_code::SERVICE_UNAVAILABLE => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = ErrorResponseBody {
            code,
            data: None,
            message: self.error.message().to_owned(),
        };
        (status, Json(body)).into_response()
    }
}
