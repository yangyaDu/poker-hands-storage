use crate::errors::AppError;
use crate::http::error_response::HttpError;

pub async fn run_blocking<T, F>(task: F) -> Result<T, HttpError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| {
            HttpError::from(AppError::invalid_format(format!(
                "Blocking request task failed: {error}"
            )))
        })?
        .map_err(HttpError::from)
}

pub async fn run_infallible_blocking<T, F>(task: F) -> Result<T, HttpError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(task).await.map_err(|error| {
        HttpError::from(AppError::invalid_format(format!(
            "Blocking request task failed: {error}"
        )))
    })
}
