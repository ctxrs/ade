use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::WorkspaceId;

use crate::daemon::{AppState, StoreLookup};

pub(crate) async fn store_for_existing_workspace_status(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<ctx_store::Store, StatusCode> {
    match state.lookup_workspace_store(workspace_id).await {
        StoreLookup::Found(store) => Ok(store),
        StoreLookup::Missing | StoreLookup::Deleting => Err(StatusCode::NOT_FOUND),
        StoreLookup::Unavailable(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
