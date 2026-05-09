use std::sync::Arc;

use crate::daemon::AppState;

pub(in crate::daemon::sessions::subagents) async fn cleanup_archived_subagent_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    child: &ctx_core::models::Session,
) -> bool {
    if child.worktree_id == parent.worktree_id {
        return false;
    }

    let mut cleanup_failed = false;
    let sharing_sessions = match store
        .list_all_sessions_for_worktree(child.worktree_id)
        .await
    {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree session references: {error:#}"
            );
            return true;
        }
    };
    if sharing_sessions
        .iter()
        .any(|session| session.id != child.id)
    {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            "archived subagent worktree is still referenced by another session"
        );
        return true;
    }

    let other_tasks = match store
        .count_tasks_for_worktree(child.worktree_id, Some(child.task_id))
        .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree task references: {error:#}"
            );
            return true;
        }
    };
    if other_tasks > 0 {
        tracing::warn!(
            parent_session_id = %parent.id.0,
            child_session_id = %child.id.0,
            worktree_id = %child.worktree_id.0,
            other_tasks,
            "archived subagent worktree is still referenced by another task"
        );
        return true;
    }

    let worktree = match store.get_worktree(child.worktree_id).await {
        Ok(Some(worktree)) => worktree,
        Ok(None) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "archived subagent worktree metadata was missing during cleanup"
            );
            return true;
        }
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %child.worktree_id.0,
                "failed to load archived subagent worktree metadata: {error:#}"
            );
            return true;
        }
    };
    let workspace = match state.global_store().get_workspace(child.workspace_id).await {
        Ok(Some(workspace)) => workspace,
        Ok(None) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                workspace_id = %child.workspace_id.0,
                "workspace not found while cleaning archived subagent worktree"
            );
            return true;
        }
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                workspace_id = %child.workspace_id.0,
                "failed to load workspace while cleaning archived subagent worktree: {error:#}"
            );
            return true;
        }
    };
    let sandbox_binding = match store.get_sandbox_binding(worktree.id).await {
        Ok(binding) => binding,
        Err(error) => {
            tracing::warn!(
                parent_session_id = %parent.id.0,
                child_session_id = %child.id.0,
                worktree_id = %worktree.id.0,
                "failed to load archived subagent sandbox binding for cleanup: {error:#}"
            );
            cleanup_failed = true;
            None
        }
    };
    let cleanup_errors = crate::api::tasks::cleanup_task_worktrees(
        state.as_ref(),
        &workspace,
        child.task_id,
        &[crate::api::tasks::TaskWorktreeCleanupTarget {
            managed_root: crate::api::tasks::managed_worktree_root(state, &workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: true,
        }],
        crate::api::tasks::BranchCleanupErrorMode::Report,
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
