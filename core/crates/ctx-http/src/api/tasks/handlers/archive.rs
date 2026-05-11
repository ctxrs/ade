use super::super::*;
use worktrees::load_archive_worktrees;

#[path = "archive/worktrees.rs"]
mod worktrees;

#[derive(Debug, Serialize)]
pub(in crate::api) struct ArchiveTaskResponse {
    #[serde(flatten)]
    pub(in crate::api) task: Task,
    pub(in crate::api) cleanup_failed: bool,
}

pub(in crate::api) async fn archive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<ArchiveTaskResponse>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task = store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let session_ids: Vec<SessionId> = store
        .list_all_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|session| session.id)
        .collect();
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let sessions = store
        .list_all_sessions_for_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    for session in &sessions {
        state.cleanup_session(session.id).await;
    }
    let worktrees = load_archive_worktrees(&store, &task, &sessions).await?;

    let updated = store
        .archive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut errors: Vec<anyhow::Error> = Vec::new();
    let mut cleanup_targets = Vec::new();
    for worktree in &worktrees {
        let other_active = match store
            .count_active_tasks_for_worktree(worktree.id, Some(task_id))
            .await
        {
            Ok(count) => count > 0,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to check worktree usage: {err:#}"
                );
                true
            }
        };
        if other_active {
            continue;
        }
        let sandbox_binding = match store.get_sandbox_binding(worktree.id).await {
            Ok(binding) => binding,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    worktree_id = %worktree.id.0,
                    "failed to load sandbox binding for cleanup: {err:#}"
                );
                None
            }
        };
        cleanup_targets.push(TaskWorktreeCleanupTarget {
            managed_root: managed_worktree_root(&state, &workspace, worktree),
            sandbox_binding,
            worktree: worktree.clone(),
            destroy_worktree_on_cleanup: true,
        });
    }
    errors.extend(
        cleanup_task_worktrees(
            state.as_ref(),
            &workspace,
            task_id,
            &cleanup_targets,
            crate::api::tasks::BranchCleanupErrorMode::Report,
        )
        .await,
    );
    let cleanup_failed = !errors.is_empty();
    if cleanup_failed {
        tracing::warn!(
            task_id = %task_id.0,
            "archive cleanup had errors after task state was persisted"
        );
    }
    let task = match store
        .get_task_with_activity(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(task) => task,
        None => return Err(StatusCode::NOT_FOUND),
    };
    let _ = state
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Archived)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    for session_id in session_ids {
        state
            .workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
    }
    Ok(Json(ArchiveTaskResponse {
        task,
        cleanup_failed,
    }))
}
