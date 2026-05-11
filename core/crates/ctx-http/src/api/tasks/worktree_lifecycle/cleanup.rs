use std::path::PathBuf;

use ctx_core::ids::TaskId;
use ctx_core::models::{SandboxBinding, Workspace, Worktree};

use super::prune_worktrees;
use crate::daemon::workspaces::vcs_hooks;
use crate::daemon::AppState;
use branches::{cleanup_collected_worktree_branches, WorktreeBranchCleanup};
use managed::cleanup_managed_worktree_target;
use sandbox::{cleanup_sandbox_materialization, SandboxCleanupOutcome};

#[path = "cleanup/branches.rs"]
mod branches;
#[path = "cleanup/managed.rs"]
mod managed;
#[path = "cleanup/sandbox.rs"]
mod sandbox;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum BranchCleanupErrorMode {
    BestEffort,
    Report,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskWorktreeCleanupTarget {
    pub worktree: Worktree,
    pub sandbox_binding: Option<SandboxBinding>,
    pub managed_root: Option<PathBuf>,
    pub destroy_worktree_on_cleanup: bool,
}

pub(crate) async fn cleanup_task_worktrees(
    state: &AppState,
    workspace: &Workspace,
    task_id: TaskId,
    targets: &[TaskWorktreeCleanupTarget],
    branch_cleanup_error_mode: BranchCleanupErrorMode,
) -> Vec<anyhow::Error> {
    let mut errors = Vec::new();
    let mut branch_cleanup = WorktreeBranchCleanup::default();
    let workspace_root_exists = tokio::fs::metadata(&workspace.root_path).await.is_ok();
    for target in targets {
        let worktree = &target.worktree;
        if let Err(err) = vcs_hooks::cleanup_worktree_hooks(state, workspace, worktree).await {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove vcs hooks: {err:#}"
            );
        }
        if let Some(binding) = target.sandbox_binding.as_ref() {
            match cleanup_sandbox_materialization(state, workspace, worktree, binding, task_id)
                .await
            {
                SandboxCleanupOutcome::Complete {
                    errors: sandbox_errors,
                } => {
                    errors.extend(sandbox_errors);
                }
                SandboxCleanupOutcome::SkipRemainingTarget { error } => {
                    errors.push(error);
                    continue;
                }
            }
        }
        if !target.destroy_worktree_on_cleanup {
            continue;
        }
        let Some(root) = target.managed_root.as_ref() else {
            continue;
        };
        cleanup_managed_worktree_target(
            workspace,
            task_id,
            worktree,
            root,
            workspace_root_exists,
            &mut branch_cleanup,
            &mut errors,
        )
        .await;
    }
    cleanup_collected_worktree_branches(
        &workspace.root_path,
        task_id,
        branch_cleanup,
        branch_cleanup_error_mode,
        &mut errors,
    )
    .await;
    errors
}
