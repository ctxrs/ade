use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::WorktreeId;
use ctx_core::models::{
    WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsSnapshot,
    WorktreeVcsTouchedFilesState,
};

use crate::daemon::state::AppState;

impl AppState {
    pub fn worktree_vcs_enabled(&self) -> bool {
        self.workspaces.worktree_vcs_enabled
    }

    pub async fn get_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        if !self.worktree_vcs_enabled() {
            return None;
        }
        if let Some(snapshot) = self.workspaces.get_worktree_vcs_snapshot(worktree_id).await {
            return Some(snapshot);
        }
        if !self.is_worktree_vcs_active(worktree_id).await {
            return None;
        }
        let store = self.store_for_worktree(worktree_id).await.ok()?;
        let mut snapshot = store
            .get_worktree_vcs_snapshot_cache(worktree_id)
            .await
            .ok()
            .flatten()?;
        snapshot.compute_state = if snapshot.summary.file_count.is_some() {
            WorktreeVcsComputeState::Ready
        } else {
            WorktreeVcsComputeState::Computing
        };
        snapshot.freshness = if snapshot.summary.file_count.is_some() {
            WorktreeVcsFreshness::Stale
        } else {
            WorktreeVcsFreshness::Refreshing
        };
        snapshot.touched_files = Default::default();
        snapshot.touched_files_state = WorktreeVcsTouchedFilesState::NotLoaded;
        self.workspaces
            .cache_worktree_vcs_snapshot(snapshot.clone())
            .await;
        Some(snapshot)
    }

    pub async fn update_worktree_vcs_activity(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        self.workspaces
            .update_worktree_vcs_activity(previous, next)
            .await;
    }

    pub async fn update_worktree_vcs_open_panes(
        &self,
        previous: &HashSet<WorktreeId>,
        next: &HashSet<WorktreeId>,
    ) {
        self.workspaces
            .update_worktree_vcs_open_panes(previous, next)
            .await;
    }

    pub async fn is_worktree_vcs_active(&self, worktree_id: WorktreeId) -> bool {
        self.workspaces.is_worktree_vcs_active(worktree_id).await
    }

    pub async fn is_worktree_vcs_pane_open(&self, worktree_id: WorktreeId) -> bool {
        self.workspaces.is_worktree_vcs_pane_open(worktree_id).await
    }

    pub async fn worktree_vcs_refresh_lock(
        &self,
        worktree_id: WorktreeId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.workspaces.worktree_vcs_refresh_lock(worktree_id).await
    }
}
