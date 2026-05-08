use ctx_core::ids::WorkspaceId;

use crate::daemon::state::{AppState, WorkspaceRuntime};

impl WorkspaceRuntime {
    pub async fn cleanup_workspace(&self, state: &AppState, workspace_id: WorkspaceId) {
        let session_ids = {
            let cache = state.sessions.session_meta_cache.lock().await;
            cache
                .iter()
                .filter_map(|(session_id, entry)| {
                    if entry.value.workspace_id == workspace_id {
                        Some(*session_id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
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
