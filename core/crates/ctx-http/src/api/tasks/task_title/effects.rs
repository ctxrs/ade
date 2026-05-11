use std::collections::HashSet;
use std::sync::Arc;

use ctx_core::ids::{TaskId, WorktreeId};
use ctx_core::models::{Task, TaskDeltaKind};
use ctx_store::Store;

use crate::daemon::AppState;

pub(super) async fn emit_task_title_update_effects(
    state: &Arc<AppState>,
    store: &Store,
    task: &Task,
) {
    let task_id = task.id;
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Updated)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    let sessions = list_task_sessions_for_web_session_cleanup(store, task_id).await;
    let worktree_id_strings =
        load_task_worktree_ids_for_web_session_cleanup(store, task, task_id, &sessions).await;
    let session_ids: HashSet<String> = sessions
        .iter()
        .map(|session| session.id.0.to_string())
        .collect();
    if let Err(e) = state
        .transport
        .web_sessions
        .close_for_task(&session_ids, &worktree_id_strings)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to close web sessions for archived task: {e:?}");
    }
}

async fn list_task_sessions_for_web_session_cleanup(
    store: &Store,
    task_id: TaskId,
) -> Vec<ctx_core::models::Session> {
    match store.list_sessions_for_task(task_id).await {
        Ok(sessions) => sessions,
        Err(e) => {
            tracing::warn!(task_id = %task_id.0, "failed to list sessions for archived task: {e:?}");
            Vec::new()
        }
    }
}

async fn load_task_worktree_ids_for_web_session_cleanup(
    store: &Store,
    task: &Task,
    task_id: TaskId,
    sessions: &[ctx_core::models::Session],
) -> HashSet<String> {
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }

    let mut worktree_id_strings = HashSet::new();
    for worktree_id in worktree_ids {
        match store.get_worktree(worktree_id).await {
            Ok(Some(worktree)) => {
                worktree_id_strings.insert(worktree.id.0.to_string());
            }
            Ok(None) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "worktree missing for archived task"
                );
            }
            Err(e) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for archived task: {e:?}"
                );
            }
        }
    }
    worktree_id_strings
}
