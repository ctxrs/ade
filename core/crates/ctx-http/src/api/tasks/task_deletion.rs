use super::*;

#[path = "task_deletion/cleanup_targets.rs"]
mod cleanup_targets;

use cleanup_targets::{
    collect_task_delete_cleanup_targets, delete_unused_worktree_records_after_cleanup,
};

pub(in crate::api) async fn delete_loaded_task_with_cleanup(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
) -> Result<(), StatusCode> {
    let sessions = store
        .list_all_sessions_for_task(task.id)
        .await
        .unwrap_or_default();
    let cleanup_targets =
        collect_task_delete_cleanup_targets(state, store, workspace, task, &sessions).await;
    for session in &sessions {
        state.cleanup_session(session.id).await;
    }
    let deleted = store
        .delete_task(task.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    let cleanup_errors = cleanup_task_worktrees(
        state.as_ref(),
        workspace,
        task.id,
        &cleanup_targets,
        crate::api::tasks::BranchCleanupErrorMode::BestEffort,
    )
    .await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            task_id = %task.id.0,
            cleanup_errors = cleanup_errors.len(),
            "delete cleanup had errors after task row removal"
        );
    }
    let cleanup_succeeded = cleanup_errors.is_empty();
    delete_unused_worktree_records_after_cleanup(
        state,
        store,
        task,
        &cleanup_targets,
        cleanup_succeeded,
    )
    .await;
    let _ = state
        .global_store()
        .delete_workspace_task_index(task.id)
        .await;
    for session in sessions {
        let _ = state
            .global_store()
            .delete_workspace_session_index(session.id)
            .await;
    }
    state
        .emit_workspace_task_delete(task.workspace_id, task.id)
        .await;
    if task.archived_at.is_some() {
        state
            .emit_workspace_archived_task_delete(task.workspace_id, task.id)
            .await;
    }
    Ok(())
}

pub(in crate::api) async fn delete_task_with_cleanup(
    state: &Arc<AppState>,
    task_id: TaskId,
) -> Result<(), StatusCode> {
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task = store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    delete_loaded_task_with_cleanup(state, &store, &workspace, &task).await
}

pub(in crate::api) async fn delete_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    delete_task_with_cleanup(&state, task_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
