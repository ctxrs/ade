use std::sync::Arc;

use ctx_core::ids::WorkspaceId;

use crate::daemon::AppState;

pub(crate) async fn load_workspace_active_snapshot_state(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
) -> (i64, i64) {
    state
        .workspaces
        .workspace_active_snapshot
        .snapshot_state(workspace_id)
        .await
}
