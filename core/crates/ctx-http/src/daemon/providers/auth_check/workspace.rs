use std::sync::Arc;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;

use crate::daemon::providers::auth_check::ProviderAuthCheckError;
use crate::daemon::AppState;

pub(super) async fn load_workspace(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> Result<Workspace, ProviderAuthCheckError> {
    state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| ProviderAuthCheckError::WorkspaceLoad)?
        .ok_or(ProviderAuthCheckError::WorkspaceNotFound)
}
