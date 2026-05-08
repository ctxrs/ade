use std::sync::Arc;

use ctx_core::ids::WorktreeId;
use ctx_core::models::Worktree;

use crate::daemon::git_status;
use crate::daemon::state::{AppState, WorkspaceRuntime};

impl WorkspaceRuntime {
    pub async fn ensure_git_status_watcher(&self, state: &Arc<AppState>, worktree: Worktree) {
        if !self.worktree_vcs_enabled {
            return;
        }
        let mut watchers = self.git_status_watchers.lock().await;
        if !watchers.insert(worktree.id) {
            return;
        }
        let worktree_id = worktree.id;
        let state = Arc::clone(state);
        tokio::spawn(async move {
            if let Err(err) = git_status::run_git_status_watcher(state.clone(), worktree).await {
                tracing::warn!(worktree_id = %worktree_id.0, "git status watcher failed: {err:#}");
            }
            state
                .workspaces
                .release_git_status_watcher(worktree_id)
                .await;
        });
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        let mut watchers = self.git_status_watchers.lock().await;
        watchers.remove(&worktree_id);
    }
}
