use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use ctx_core::ids::{TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Task, TaskDeltaKind, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree,
    WorktreeVcsSnapshot,
};
use ctx_lsp::Language as LspLanguage;

use crate::daemon::state::AppState;

use super::WorkspaceHydrationError;

impl AppState {
    pub fn worktree_vcs_enabled(&self) -> bool {
        self.workspaces.worktree_vcs_enabled
    }

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
            ctx_core::models::WorktreeVcsComputeState::Ready
        } else {
            ctx_core::models::WorktreeVcsComputeState::Computing
        };
        snapshot.freshness = if snapshot.summary.file_count.is_some() {
            ctx_core::models::WorktreeVcsFreshness::Stale
        } else {
            ctx_core::models::WorktreeVcsFreshness::Refreshing
        };
        snapshot.touched_files = ctx_core::models::WorktreeVcsTouchedFiles::default();
        snapshot.touched_files_state = ctx_core::models::WorktreeVcsTouchedFilesState::NotLoaded;
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

    pub async fn ensure_lsp_diagnostics_forwarder(
        self: &Arc<Self>,
        root: PathBuf,
        lang: LspLanguage,
    ) {
        if !self.core.lsp.enabled() {
            return;
        }
        let key = format!("{}:{}", root.to_string_lossy(), lang.id());
        {
            let mut set = self.transport.lsp_diag_forwarders.lock().await;
            if set.contains(&key) {
                return;
            }
            set.insert(key);
        }

        let state = self.clone();
        tokio::spawn(async move {
            let mut rx = match state
                .core
                .lsp
                .subscribe_diagnostics_for_language(&root, lang)
                .await
            {
                Ok(v) => v,
                Err(_) => return,
            };
            loop {
                let update = match rx.recv().await {
                    Ok(u) => u,
                    Err(_) => break,
                };
                let url = match url::Url::parse(&update.uri.to_string()) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                if url.scheme() != "file" {
                    continue;
                }
                let Ok(abs_path) = url.to_file_path() else {
                    continue;
                };

                let watchers = state.core.buffers.watchers_for_abs_path(&abs_path).await;
                if watchers.is_empty() {
                    continue;
                }

                let diagnostics_json = match serde_json::to_value(&update.diagnostics) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                for (sid, rel) in watchers {
                    let msg = serde_json::json!({
                        "type": "lsp_diagnostics",
                        "session_id": sid.0.to_string(),
                        "path": rel.to_string_lossy(),
                        "diagnostics": diagnostics_json,
                    });
                    let _ = state.transport.lsp_diag_broadcaster.send(msg);
                }
            }
        });
    }
}
