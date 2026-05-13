use std::sync::Arc;

use axum::http::StatusCode;
use ctx_core::ids::WorkspaceId;

use crate::daemon::{AppState, WorkspaceStoreAccessError};

pub(crate) async fn store_for_existing_workspace_status(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<ctx_store::Store, StatusCode> {
    state
        .existing_workspace_store(workspace_id)
        .await
        .map_err(workspace_store_status)
}

fn workspace_store_status(error: WorkspaceStoreAccessError) -> StatusCode {
    match error {
        WorkspaceStoreAccessError::NotFound => StatusCode::NOT_FOUND,
        WorkspaceStoreAccessError::Unavailable(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
