use std::sync::Arc;
use std::time::Instant;

use crate::query::QueryService;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<QueryService>,
    pub started_at: Instant,
}
