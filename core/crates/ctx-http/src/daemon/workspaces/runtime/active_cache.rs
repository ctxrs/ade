use ctx_core::ids::WorkspaceId;
use ctx_core::models::{WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot};
use ctx_workspace_active_snapshot::{
    WorkspaceActiveHeadCacheEntry, WorkspaceActiveSnapshotCacheEntry,
};

use crate::daemon::state::{TimedEntry, WorkspaceRuntime};

impl WorkspaceRuntime {
    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            (
                entry.value.snapshot.snapshot_rev,
                entry.value.snapshot.archived_rev,
            )
        })
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.snapshot.clone()
        })
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        let workspace_id = snapshot.workspace_id;
        let mut cache = self.workspace_active_snapshot_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveSnapshotCacheEntry { snapshot }),
        );
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.get_mut(&workspace_id).map(|entry| {
            entry.touch();
            entry.value.batch.clone()
        })
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        let workspace_id = batch.workspace_id;
        let mut cache = self.workspace_active_heads_cache.lock().await;
        cache.insert(
            workspace_id,
            TimedEntry::new(WorkspaceActiveHeadCacheEntry { batch }),
        );
    }
}
