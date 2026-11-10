//! TODO: port from Plane. See WORKERS.md.
use axum::Router;

use crate::common::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
}
