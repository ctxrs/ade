use super::*;

#[path = "worktree_lifecycle/cleanup.rs"]
mod cleanup;
#[path = "worktree_lifecycle/git_ops.rs"]
mod git_ops;
#[path = "worktree_lifecycle/persistence.rs"]
mod persistence;
#[path = "worktree_lifecycle/retry.rs"]
mod retry;
#[path = "worktree_lifecycle/sandbox_binding.rs"]
mod sandbox_binding;

pub(crate) use cleanup::{
    cleanup_task_worktrees, BranchCleanupErrorMode, TaskWorktreeCleanupTarget,
};
#[cfg(test)]
pub(crate) use git_ops::branch_exists;
pub(crate) use git_ops::ensure_worktree_attached;
pub(super) use git_ops::{is_git_worktree, prune_worktrees, remove_worktree};
pub(crate) use persistence::{
    managed_worktree_root, persist_provisioned_worktree, provision_worktree_for_execution,
};
pub(crate) use retry::retry_global_index_write;
pub(crate) use sandbox_binding::{
    execution_environment_from_settings, rematerialize_sandbox_binding_for_worktree,
};
