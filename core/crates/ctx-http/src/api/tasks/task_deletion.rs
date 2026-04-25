use super::*;

pub(in crate::api) async fn delete_loaded_task_with_cleanup(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task: &Task,
) -> Result<(), StatusCode> {
    let sessions = store
        .list_sessions_for_task(task.id)
        .await
        .unwrap_or_default();
    let mut worktree_ids: HashSet<WorktreeId> = sessions.iter().map(|s| s.worktree_id).collect();
    if let Some(primary_worktree_id) = task.primary_worktree_id {
        worktree_ids.insert(primary_worktree_id);
    }
    let mut cleanup_targets = Vec::new();
    for worktree_id in &worktree_ids {
        let other_active = match store
            .count_active_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check worktree usage: {err:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        let other_tasks = match store
            .count_tasks_for_worktree(*worktree_id, Some(task.id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to check total worktree usage: {err:#}"
                );
                true
            }
        };
        let worktree = match store.get_worktree(*worktree_id).await {
            Ok(Some(worktree)) => worktree,
            Ok(None) => continue,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load worktree for delete cleanup: {err:#}"
                );
                continue;
            }
        };
        let sandbox_binding = match store.get_sandbox_binding(*worktree_id).await {
            Ok(binding) => binding,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "failed to load sandbox binding for delete cleanup: {err:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: managed_worktree_root(&state, &workspace, &worktree),
            sandbox_binding,
            worktree,
            destroy_worktree_on_cleanup: !other_tasks,
        });
    }
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
    let cleanup_errors =
        cleanup_task_worktrees(state.as_ref(), workspace, task.id, &cleanup_targets).await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            task_id = %task.id.0,
            cleanup_errors = cleanup_errors.len(),
            "delete cleanup had errors after task row removal"
        );
    }
    let cleanup_succeeded = cleanup_errors.is_empty();
    for target in &cleanup_targets {
        if !target.destroy_worktree_on_cleanup || !cleanup_succeeded {
            continue;
        }
        let deleted_worktree_row = match store.delete_worktree(target.worktree.id).await {
            Ok(deleted) => deleted,
            Err(err) => {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %target.worktree.id.0,
                    "failed to delete worktree row after task delete: {err:#}"
                );
                false
            }
        };
        if !deleted_worktree_row {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "skipping worktree index deletion because worktree row was not deleted"
            );
            continue;
        }
        if let Err(err) = state
            .global_store()
            .delete_workspace_worktree_index(target.worktree.id)
            .await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %target.worktree.id.0,
                "failed to delete worktree index after task delete: {err:#}"
            );
        }
    }
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
