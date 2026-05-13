use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::WorktreeId;
use ctx_core::models::{
    WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsSnapshot, WorktreeVcsTouchedFiles,
    WorktreeVcsTouchedFilesState,
};
use ctx_workspace_services::worktree_vcs::{
    GitStatusSnapshot, WorktreeVcsDirtyBits, WorktreeVcsSchedulerJob,
};
use tokio::sync::{broadcast, OwnedSemaphorePermit};

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

    pub async fn queue_worktree_vcs_refresh(
        &self,
        worktree_id: WorktreeId,
        summary: bool,
        touched_files: bool,
    ) {
        self.workspaces
            .queue_worktree_vcs_refresh(worktree_id, summary, touched_files)
            .await;
    }

    pub async fn mark_worktree_vcs_runtime_dirty(
        &self,
        worktree_id: WorktreeId,
        dirty_bits: WorktreeVcsDirtyBits,
        candidate_paths: Vec<String>,
        pane_open: bool,
    ) {
        self.workspaces
            .mark_worktree_vcs_dirty(worktree_id, dirty_bits, candidate_paths, pane_open)
            .await;
    }

    pub async fn finish_worktree_vcs_refresh(
        &self,
        worktree_id: WorktreeId,
        git_snapshot: GitStatusSnapshot,
        touched_files: WorktreeVcsTouchedFiles,
        touched_files_state: WorktreeVcsTouchedFilesState,
    ) {
        self.workspaces
            .finish_worktree_vcs_refresh(
                worktree_id,
                git_snapshot,
                touched_files,
                touched_files_state,
            )
            .await;
    }

    pub async fn upsert_worktree_vcs_snapshot(
        &self,
        snapshot: WorktreeVcsSnapshot,
        force_emit: bool,
        summary_at: Option<std::time::Instant>,
    ) -> Option<WorktreeVcsSnapshot> {
        self.workspaces
            .upsert_worktree_vcs_snapshot(snapshot, force_emit, summary_at)
            .await
    }

    pub fn publish_worktree_vcs_event(&self, snapshot: WorktreeVcsSnapshot) {
        self.workspaces.publish_worktree_vcs_event(snapshot);
    }

    pub fn subscribe_worktree_vcs_events(&self) -> broadcast::Receiver<WorktreeVcsSnapshot> {
        self.workspaces.subscribe_worktree_vcs_events()
    }

    pub async fn claim_next_worktree_vcs_job(&self) -> Option<WorktreeVcsSchedulerJob> {
        self.workspaces.claim_next_worktree_vcs_job().await
    }

    pub async fn finish_worktree_vcs_job(&self, worktree_id: WorktreeId) -> bool {
        self.workspaces.finish_worktree_vcs_job(worktree_id).await
    }

    pub async fn wait_worktree_vcs_scheduler_notification(&self) {
        self.workspaces
            .wait_worktree_vcs_scheduler_notification()
            .await;
    }

    pub fn notify_worktree_vcs_scheduler(&self) {
        self.workspaces.notify_worktree_vcs_scheduler();
    }

    pub fn try_acquire_worktree_vcs_scheduler_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.workspaces.try_acquire_worktree_vcs_scheduler_permit()
    }

    pub fn mark_worktree_vcs_scheduler_started(&self) -> bool {
        self.workspaces.mark_worktree_vcs_scheduler_started()
    }
}
