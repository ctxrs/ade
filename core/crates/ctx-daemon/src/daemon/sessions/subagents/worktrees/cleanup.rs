use std::sync::Arc;

use crate::daemon::workspaces::{
    cleanup_task_worktrees, managed_worktree_root, BranchCleanupErrorMode,
    TaskWorktreeCleanupTarget,
};
use crate::daemon::DaemonState;

mod context;
mod references;

use context::load_archived_worktree_cleanup_context;
use references::archived_worktree_has_other_references;

pub(in crate::daemon::sessions::subagents) async fn cleanup_archived_subagent_worktree(
    state: &Arc<DaemonState>,
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    child: &ctx_core::models::Session,
) -> bool {
    if child.worktree_id == parent.worktree_id {
        return false;
    }

    if archived_worktree_has_other_references(store, parent, child).await {
        return true;
    }

    let Some(cleanup_context) =
        load_archived_worktree_cleanup_context(state, store, parent, child).await
    else {
        return true;
    };
    let mut cleanup_failed = cleanup_context.cleanup_failed;
    let cleanup_errors = cleanup_task_worktrees(
        state.as_ref(),
        &cleanup_context.workspace,
        child.task_id,
        &[TaskWorktreeCleanupTarget {
            managed_root: managed_worktree_root(
                state.as_ref(),
                &cleanup_context.workspace,
                &cleanup_context.worktree,
            ),
            sandbox_binding: cleanup_context.sandbox_binding,
            worktree: cleanup_context.worktree,
            destroy_worktree_on_cleanup: true,
        }],
        BranchCleanupErrorMode::Report,
    )
    .await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            cleanup_errors = cleanup_errors.len(),
            "archived subagent worktree cleanup had errors"
        );
        cleanup_failed = true;
    }

    if !cleanup_failed {
        if let Err(error) = store.delete_sandbox_binding(child.worktree_id).await {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to delete archived subagent sandbox binding after cleanup: {error:#}"
            );
            cleanup_failed = true;
        }
    }
    cleanup_failed
}
