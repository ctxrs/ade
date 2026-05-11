use std::path::Path;

use anyhow::Context;
use ctx_core::ids::TaskId;
use ctx_core::models::{Workspace, Worktree};

use super::super::{is_git_worktree, remove_worktree};
use super::branches::{collect_worktree_branch_for_cleanup, WorktreeBranchCleanup};

pub(super) async fn cleanup_managed_worktree_target(
    workspace: &Workspace,
    task_id: TaskId,
    worktree: &Worktree,
    root: &Path,
    workspace_root_exists: bool,
    branch_cleanup: &mut WorktreeBranchCleanup,
    errors: &mut Vec<anyhow::Error>,
) {
    let branch = worktree
        .git_branch
        .as_deref()
        .filter(|name| name.starts_with("ctx/"));
    if !workspace_root_exists {
        remove_orphaned_worktree_dir(workspace, task_id, worktree, root, errors).await;
        return;
    }
    if tokio::fs::metadata(root).await.is_err() {
        if branch.is_some() {
            branch_cleanup.mark_needs_prune();
        }
        collect_branch_if_present(branch_cleanup, branch);
        return;
    }
    let embedded_git_dir = tokio::fs::metadata(root.join(".git"))
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false);
    let is_git = embedded_git_dir || is_git_worktree(root).await.unwrap_or(false);
    let should_collect_branch = if embedded_git_dir {
        remove_standalone_managed_worktree(task_id, worktree, root, branch_cleanup, errors).await;
        true
    } else if is_git {
        remove_git_worktree(workspace, task_id, worktree, root, branch_cleanup, errors).await
    } else {
        remove_non_git_worktree_dir(task_id, worktree, root, errors).await;
        true
    };
    if should_collect_branch {
        collect_branch_if_present(branch_cleanup, branch);
    }
}

async fn remove_orphaned_worktree_dir(
    workspace: &Workspace,
    task_id: TaskId,
    worktree: &Worktree,
    root: &Path,
    errors: &mut Vec<anyhow::Error>,
) {
    if tokio::fs::metadata(root).await.is_err() {
        return;
    }
    if let Err(err) = tokio::fs::remove_dir_all(root)
        .await
        .with_context(|| format!("removing orphaned worktree dir at {}", root.display()))
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            workspace_root = %workspace.root_path,
            "failed to remove orphaned worktree dir after workspace root disappeared: {err:#}"
        );
        errors.push(err);
    }
}

async fn remove_standalone_managed_worktree(
    task_id: TaskId,
    worktree: &Worktree,
    root: &Path,
    branch_cleanup: &mut WorktreeBranchCleanup,
    errors: &mut Vec<anyhow::Error>,
) {
    branch_cleanup.mark_needs_prune();
    if let Err(err) = tokio::fs::remove_dir_all(root)
        .await
        .with_context(|| format!("removing standalone managed worktree at {}", root.display()))
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            "failed to remove standalone managed worktree dir: {err:#}"
        );
        errors.push(err);
    }
}

async fn remove_git_worktree(
    workspace: &Workspace,
    task_id: TaskId,
    worktree: &Worktree,
    root: &Path,
    branch_cleanup: &mut WorktreeBranchCleanup,
    errors: &mut Vec<anyhow::Error>,
) -> bool {
    branch_cleanup.mark_needs_prune();
    if let Err(err) = remove_worktree(&workspace.root_path, root).await {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            "failed to remove worktree: {err:#}"
        );
        errors.push(err);
        return false;
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
    true
}

async fn remove_non_git_worktree_dir(
    task_id: TaskId,
    worktree: &Worktree,
    root: &Path,
    errors: &mut Vec<anyhow::Error>,
) {
    if let Err(err) = tokio::fs::remove_dir_all(root)
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
}

fn collect_branch_if_present(branch_cleanup: &mut WorktreeBranchCleanup, branch: Option<&str>) {
    if let Some(branch) = branch {
        collect_worktree_branch_for_cleanup(branch_cleanup, branch);
    }
}
