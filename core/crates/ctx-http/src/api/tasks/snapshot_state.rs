use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::ids::WorkspaceId;

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
