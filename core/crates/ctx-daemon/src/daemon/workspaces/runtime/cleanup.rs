use ctx_core::ids::WorkspaceId;

use crate::daemon::state::{DaemonState, WorkspaceRuntime};

impl WorkspaceRuntime {
    pub async fn cleanup_workspace(&self, state: &DaemonState, workspace_id: WorkspaceId) {
        let session_ids = state.cached_session_ids_for_workspace(workspace_id).await;
        for session_id in session_ids {
            state.cleanup_session(session_id).await;
        }
        {
            let mut cache = self.workspace_active_snapshot_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_active_heads_cache.lock().await;
            cache.remove(&workspace_id);
        }
        {
            let mut cache = self.workspace_file_completions_cache.lock().await;
            cache.remove(&workspace_id);
        }
        self.workspace_active_snapshot
            .remove_workspace(workspace_id)
            .await;
        state.core.stores.evict_workspace(workspace_id).await;
    }
}
