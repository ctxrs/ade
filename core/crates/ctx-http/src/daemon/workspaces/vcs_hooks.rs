#[path = "vcs_hooks/host.rs"]
mod host;
#[path = "vcs_hooks/sandbox.rs"]
mod sandbox;

use anyhow::Result;
use ctx_core::ids::TaskId;
use ctx_core::models::{Workspace, Worktree};
pub(crate) use ctx_workspace_services::vcs_hooks::cleanup_workspace_hooks;

use crate::daemon::AppState;

pub async fn ensure_task_commit_hook(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
    task_id: TaskId,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::ensure_task_commit_hook(state, workspace, worktree, task_id)
        .await
}

pub async fn cleanup_worktree_hooks(
    state: &AppState,
    workspace: &Workspace,
    worktree: &Worktree,
) -> Result<()> {
    ctx_workspace_services::vcs_hooks::cleanup_worktree_hooks(state, workspace, worktree).await
}

#[cfg(test)]
mod tests;
