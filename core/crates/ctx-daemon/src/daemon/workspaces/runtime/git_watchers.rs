use ctx_core::ids::WorktreeId;
use ctx_core::models::Worktree;

use crate::daemon::git_status::{WorktreeVcsExecutionHost, WorktreeVcsRuntimeHost};
use crate::daemon::state::WorkspaceRuntime;

impl WorkspaceRuntime {
    pub(in crate::daemon) async fn ensure_git_status_watcher(
        &self,
        runtime: WorktreeVcsRuntimeHost,
        execution: WorktreeVcsExecutionHost,
        worktree: Worktree,
    ) {
        runtime.ensure_git_status_watcher(execution, worktree).await;
    }

    pub async fn release_git_status_watcher(&self, worktree_id: WorktreeId) {
        let mut watchers = self.git_status_watchers.lock().await;
        watchers.remove(&worktree_id);
    }
}
