use serde::Serialize;
use utoipa::ToSchema;

/// Unified success response envelope: `{ code, data, message }`.
/// `code == 0` means success; non-zero means a business error.
#[derive(Serialize, ToSchema)]
pub struct ApiResponse<T> {
    /// Business status code. 0 indicates success.
    code: i32,
    /// Typed response payload (null on error).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<T>)]
    data: Option<T>,
    /// Human-readable message. Success responses use null; error responses carry a message.
    #[schema(nullable)]
    message: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            code: 0,
            data: Some(data),
            message: None,
        }
    }
}
