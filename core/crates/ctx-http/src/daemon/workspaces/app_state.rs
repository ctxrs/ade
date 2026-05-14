use std::sync::Arc;

use anyhow::Result;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Task, TaskDeltaKind, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree,
};

use crate::daemon::state::DaemonState;

use super::{WorkspaceCacheDebugStats, WorkspaceHydrationError};

mod worktree_vcs;

impl DaemonState {
    pub async fn cached_workspace_active_snapshot_state(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<(i64, i64)> {
        self.workspaces
            .cached_workspace_active_snapshot_state(workspace_id)
            .await
    }

    pub async fn cached_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveSnapshot> {
        self.workspaces
            .cached_workspace_active_snapshot(workspace_id)
            .await
    }

    pub async fn cache_workspace_active_snapshot(&self, snapshot: WorkspaceActiveSnapshot) {
        self.workspaces
            .cache_workspace_active_snapshot(snapshot)
            .await;
    }

    pub async fn cached_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Option<WorkspaceActiveHeadBatch> {
        self.workspaces
            .cached_workspace_active_heads(workspace_id)
            .await
    }

    pub async fn cache_workspace_active_heads(&self, batch: WorkspaceActiveHeadBatch) {
        self.workspaces.cache_workspace_active_heads(batch).await;
    }

    pub async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> std::result::Result<(), WorkspaceHydrationError> {
        self.workspaces
            .ensure_workspace_active_snapshot_hydrated(self, workspace_id)
            .await
    }

    pub async fn register_worktree_bootstrap(
        &self,
        worktree_id: WorktreeId,
        wait_for_completion: bool,
    ) {
        self.workspaces
            .register_worktree_bootstrap(worktree_id, wait_for_completion)
            .await;
    }

    pub async fn finish_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        self.workspaces.finish_worktree_bootstrap(worktree_id).await;
    }

    pub async fn wait_for_worktree_bootstrap(&self, worktree_id: WorktreeId) {
        self.workspaces
            .wait_for_worktree_bootstrap(worktree_id)
            .await;
    }

    pub async fn cleanup_workspace(&self, workspace_id: WorkspaceId) {
        // Best-effort: workspace deletion should attempt to clean up its harness container + volume,
        // but must not fail deletion if the sandbox container runtime is unavailable.
        let _ = self.execution.harness.stop_container(workspace_id).await;
        let _ = self
            .execution
            .harness
            .remove_workspace_volume(workspace_id)
            .await;
        self.workspaces.cleanup_workspace(self, workspace_id).await;
    }

    pub async fn emit_workspace_task_upsert(&self, task_id: TaskId) -> Result<()> {
        self.workspaces
            .emit_workspace_task_upsert(self, task_id)
            .await
    }

    pub async fn emit_workspace_task_delta(&self, task: Task, kind: TaskDeltaKind) -> bool {
        self.workspaces
            .workspace_active_snapshot
            .publish_task_delta(task.workspace_id, task, kind)
            .await
    }

    pub async fn emit_workspace_task_delete(&self, workspace_id: WorkspaceId, task_id: TaskId) {
        self.workspaces
            .emit_workspace_task_delete(self, workspace_id, task_id)
            .await;
    }

    pub async fn emit_workspace_archived_task_delete(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        self.workspaces
            .emit_workspace_archived_task_delete(self, workspace_id, task_id)
            .await;
    }

    pub async fn ensure_git_status_watcher(self: &Arc<Self>, worktree: Worktree) {
        self.workspaces
            .ensure_git_status_watcher(self, worktree)
            .await;
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        self.workspaces
            .release_git_status_watcher(worktree_id)
            .await;
    }

    pub(crate) async fn workspace_cache_debug_stats(&self) -> WorkspaceCacheDebugStats {
        self.workspaces.cache_debug_stats().await
    }
}
