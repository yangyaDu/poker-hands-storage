use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::Json;
use serde::de::DeserializeOwned;
use serde::Serialize;
use utoipa::ToSchema;

use crate::http::HttpError;

// Allowed dimension values.
pub const ALLOWED_STRATEGIES: &[&str] = &["default"];
pub const ALLOWED_PLAYER_COUNTS: &[u32] = &[6, 8, 9];
pub const ALLOWED_DEPTH_BB: &[u32] = &[100, 200, 300];

pub const MAX_BATCH_REQUESTS: usize = 500;
pub const MAX_PREWARM_DIMENSIONS: usize = 64;

pub trait ValidateRequest {
    fn validate(&self) -> Result<(), ValidationErrorDetails>;
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FieldValidationError {
    /// JSON field path that failed validation.
    pub path: String,
    /// Human-readable validation message.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ValidationErrorDetails {
    /// Field-level validation errors.
    pub fields: Vec<FieldValidationError>,
}

impl ValidationErrorDetails {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn push(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.fields.push(FieldValidationError {
            path: path.into(),
            message: message.into(),
        });
    }

    pub fn finish(self) -> Result<(), Self> {
        if self.fields.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }

    pub fn message(&self) -> String {
        self.fields
            .iter()
            .map(|field| format!("{} {}", field.path, field.message))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl Default for ValidationErrorDetails {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + ValidateRequest,
{
    type Rejection = HttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state)
            .await
            .map_err(json_rejection)?;
        value.validate().map_err(HttpError::validation)?;
        Ok(Self(value))
    }
}

fn json_rejection(error: JsonRejection) -> HttpError {
    HttpError::invalid_json(error.body_text())
}

pub fn validate_required_string(
    errors: &mut ValidationErrorDetails,
    path: impl Into<String>,
    value: &str,
) {
    if value.trim().is_empty() {
        errors.push(path, "must not be empty");
    }
}

pub fn validate_positive_u32(
    errors: &mut ValidationErrorDetails,
    path: impl Into<String>,
    value: u32,
) {
    if value == 0 {
        errors.push(path, "must be greater than 0");
    }
}

pub fn validate_allowed_str(
    errors: &mut ValidationErrorDetails,
    path: impl Into<String>,
    value: &str,
    allowed: &[&str],
    message: impl Into<String>,
) {
    if !allowed.contains(&value) {
        errors.push(path, message);
    }
}

pub fn validate_allowed_u32(
    errors: &mut ValidationErrorDetails,
    path: impl Into<String>,
    value: u32,
    allowed: &[u32],
    message: impl Into<String>,
) {
    if !allowed.contains(&value) {
        errors.push(path, message);
    }
}
