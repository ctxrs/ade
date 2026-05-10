use std::path::PathBuf;

use anyhow::Context;
use ctx_core::ids::TaskId;
use ctx_core::models::{SandboxBinding, Workspace, Worktree};

use super::{is_git_worktree, prune_worktrees, remove_worktree};
use crate::daemon::workspaces::vcs_hooks;
use crate::daemon::AppState;
use ctx_fs::git::delete_branch;
use sandbox::{cleanup_sandbox_materialization, SandboxCleanupOutcome};

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
    let mut needs_prune = false;
    let mut branches_to_delete = Vec::new();
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
        let branch = worktree
            .git_branch
            .as_deref()
            .filter(|name| name.starts_with("ctx/"));
        if !workspace_root_exists {
            if tokio::fs::metadata(root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(root).await.with_context(|| {
                    format!("removing orphaned worktree dir at {}", root.display())
                }) {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        workspace_root = %workspace.root_path,
                        "failed to remove orphaned worktree dir after workspace root disappeared: {err:#}"
                    );
                    errors.push(err);
                }
            }
            continue;
        }
        if tokio::fs::metadata(root).await.is_err() {
            if branch.is_some() {
                needs_prune = true;
            }
            if let Some(branch) = branch {
                branches_to_delete.push(branch.to_string());
            }
            continue;
        }
        let embedded_git_dir = tokio::fs::metadata(root.join(".git"))
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false);
        let is_git = embedded_git_dir || is_git_worktree(root).await.unwrap_or(false);
        if embedded_git_dir {
            needs_prune = true;
            if let Err(err) = tokio::fs::remove_dir_all(root).await.with_context(|| {
                format!("removing standalone managed worktree at {}", root.display())
            }) {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove standalone managed worktree dir: {err:#}"
                );
                errors.push(err);
            }
        } else if is_git {
            needs_prune = true;
            if let Err(err) = remove_worktree(&workspace.root_path, root).await {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to remove worktree: {err:#}"
                );
                errors.push(err);
                continue;
            }
            if tokio::fs::metadata(root).await.is_ok() {
                if let Err(err) = tokio::fs::remove_dir_all(root)
                    .await
                    .with_context(|| format!("removing worktree dir at {}", root.display()))
                {
                    tracing::warn!(
                        task_id = %task_id.0,
                        worktree_id = %worktree.id.0,
                        "failed to remove worktree dir: {err:#}"
                    );
                    errors.push(err);
                }
            }
        } else if let Err(err) = tokio::fs::remove_dir_all(root)
            .await
            .with_context(|| format!("removing non-git worktree dir at {}", root.display()))
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to remove worktree dir: {err:#}"
            );
            errors.push(err);
        }
        if let Some(branch) = branch {
            branches_to_delete.push(branch.to_string());
        }
    }
    if needs_prune {
        if let Err(err) = prune_worktrees(&workspace.root_path).await {
            tracing::warn!(task_id = %task_id.0, "failed to prune worktrees: {err:#}");
            errors.push(err);
        }
    }
    branches_to_delete.sort();
    branches_to_delete.dedup();
    for branch in branches_to_delete {
        if let Err(err) = delete_branch(&workspace.root_path, &branch).await {
            tracing::warn!(
                task_id = %task_id.0,
                branch,
                "failed to delete worktree branch: {err:#}"
            );
            if matches!(branch_cleanup_error_mode, BranchCleanupErrorMode::Report) {
                errors.push(err);
            }
        }
    }
    errors
}
