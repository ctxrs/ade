mod active_snapshot_state;
mod app_state;
pub(crate) mod attachments;
mod diff_exec;
mod execution;
mod hydration;
mod retry;
mod runtime;
mod sandbox_binding;
pub(crate) mod stream;
pub(crate) mod vcs_hooks;
mod worktree_bootstrap;
mod worktree_cleanup;
mod worktree_provision;

pub(crate) use active_snapshot_state::load_workspace_active_snapshot_state;
pub(crate) use diff_exec::{diff_worktree_for_session, diff_worktree_summary_for_session};
pub(crate) use execution::{
    execution_environment_from_settings, resolve_existing_worktree_execution,
};
pub use hydration::{WorkspaceHydrationError, WorkspaceHydrationErrorKind};
pub(crate) use retry::retry_global_index_write;
pub(crate) use sandbox_binding::rematerialize_sandbox_binding_for_worktree;
pub(crate) use worktree_bootstrap::spawn_worktree_bootstrap;
pub(crate) use worktree_cleanup::{
    cleanup_task_worktrees, managed_worktree_root, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
pub(crate) use worktree_provision::{
    persist_provisioned_worktree, provision_worktree_for_execution,
};
