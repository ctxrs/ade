use super::*;

#[path = "task_deletion/cleanup_targets.rs"]
mod cleanup_targets;

use cleanup_targets::{
    collect_task_delete_cleanup_targets, delete_unused_worktree_records_after_cleanup,
};

pub(in crate::api) async fn delete_loaded_task_with_cleanup(
    handles: &TaskApiHandles,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
) -> Result<(), StatusCode> {
    let sessions = store
        .list_all_sessions_for_task(task.id)
        .await
        .unwrap_or_default();
    let cleanup_targets =
        collect_task_delete_cleanup_targets(handles, store, workspace, task, &sessions).await;
    for session in &sessions {
        handles.sessions.cleanup_session(session.id).await;
    }
    let deleted = store
        .delete_task(task.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !deleted {
        return Err(StatusCode::NOT_FOUND);
    }
    let cleanup_errors = handles
        .workspaces
        .cleanup_task_worktrees(
            workspace,
            task.id,
            &cleanup_targets,
            BranchCleanupErrorMode::BestEffort,
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
        handles,
        store,
        task,
        &cleanup_targets,
        cleanup_succeeded,
    )
    .await;
    let _ = handles.sessions.delete_workspace_task_index(task.id).await;
    for session in sessions {
        let _ = handles
            .sessions
            .delete_workspace_session_index(session.id)
            .await;
    }
    handles
        .workspaces
        .emit_workspace_task_delete(task.workspace_id, task.id)
        .await;
    if task.archived_at.is_some() {
        handles
            .workspaces
            .emit_workspace_archived_task_delete(task.workspace_id, task.id)
            .await;
    }
    Ok(())
}

pub(in crate::api) async fn delete_task(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    tasks
        .delete_task(task_id)
        .await
        .map_err(task_lifecycle_status)?;
    Ok(StatusCode::NO_CONTENT)
}
