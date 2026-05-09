use std::sync::Arc;

use axum::routing::{get, post};

use crate::api::providers::{get_provider, get_provider_usage, list_providers};
use crate::api::refresh_provider_matrix;
use crate::daemon::AppState;

pub(super) fn provider_base_routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/providers", get(list_providers))
        .route(
            "/api/providers/matrix/refresh",
            post(refresh_provider_matrix),
        )
        .route("/api/providers/:id", get(get_provider))
        .route("/api/providers/:id/usage", get(get_provider_usage))
}
