use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::Worktree;

use super::{internal_error, not_found, TerminalLaunchError};

pub(super) async fn resolve_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    worktree_id: Option<WorktreeId>,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Result<Option<Worktree>, TerminalLaunchError> {
    if let Some(worktree_id) = worktree_id {
        return load_explicit_worktree(state, workspace_id, worktree_id).await;
    }
    if session_id.is_some() || task_id.is_some() {
        return infer_terminal_worktree(state, workspace_id, session_id, task_id).await;
    }
    Ok(None)
}

async fn load_explicit_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
) -> Result<Option<Worktree>, TerminalLaunchError> {
    let store = state
        .store_for_worktree(worktree_id)
        .await
        .map_err(|_| not_found("worktree not found"))?;
    let worktree = store
        .get_worktree(worktree_id)
        .await
        .map_err(|_| internal_error("failed to load worktree"))?
        .ok_or_else(|| not_found("worktree not found"))?;
    if worktree.workspace_id != workspace_id {
        return Err(not_found("worktree not found"));
    }
    Ok(Some(worktree))
}

pub(crate) async fn infer_terminal_worktree(
    state: &Arc<AppState>,
    workspace_id: WorkspaceId,
    session_id: Option<SessionId>,
    task_id: Option<TaskId>,
) -> Result<Option<Worktree>, TerminalLaunchError> {
    if let Some(session_id) = session_id {
        let store = state
            .store_for_session(session_id)
            .await
            .map_err(|_| not_found("session not found"))?;
        let session = store
            .get_session(session_id)
            .await
            .map_err(|_| internal_error("failed to load session"))?
            .ok_or_else(|| not_found("session not found"))?;
        if session.workspace_id != workspace_id {
            return Err(not_found("session not found"));
        }
        let worktree = store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|_| internal_error("failed to load worktree"))?
            .ok_or_else(|| not_found("worktree not found"))?;
        if worktree.workspace_id != workspace_id {
            return Err(not_found("worktree not found"));
        }
        return Ok(Some(worktree));
    }

    if let Some(task_id) = task_id {
        let store = state
            .store_for_task(task_id)
            .await
            .map_err(|_| not_found("task not found"))?;
        let task = store
            .get_task(task_id)
            .await
            .map_err(|_| internal_error("failed to load task"))?
            .ok_or_else(|| not_found("task not found"))?;
        if task.workspace_id != workspace_id {
            return Err(not_found("task not found"));
        }
        let primary_worktree_id = task
            .primary_worktree_id
            .ok_or_else(|| not_found("worktree not found"))?;
        let worktree = store
            .get_worktree(primary_worktree_id)
            .await
            .map_err(|_| internal_error("failed to load worktree"))?
            .ok_or_else(|| not_found("worktree not found"))?;
        if worktree.workspace_id != workspace_id {
            return Err(not_found("worktree not found"));
        }
        return Ok(Some(worktree));
    }

    if let Ok(store) = state.store_for_workspace(workspace_id).await {
        if let Ok(worktrees) = store.list_worktrees(workspace_id).await {
            return Ok(worktrees.into_iter().last());
        }
    }

    Ok(None)
}
