#[cfg(test)]
pub(crate) use ctx_workspace_services::worktree_vcs::branch_exists;
pub(crate) use ctx_workspace_services::worktree_vcs::{
    ensure_worktree_attached, is_git_worktree, prune_worktrees, remove_worktree,
};
