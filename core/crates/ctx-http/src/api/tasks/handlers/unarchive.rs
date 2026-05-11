use super::super::*;
use worktrees::{load_unarchive_worktree_plan, UnarchiveWorktreePlan};

#[path = "unarchive/worktrees.rs"]
mod worktrees;

pub(in crate::api) async fn unarchive_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
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
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let UnarchiveWorktreePlan {
        session_ids,
        managed_worktrees,
        worktrees,
    } = load_unarchive_worktree_plan(&state, &store, &workspace, &task).await?;

    for (worktree, root) in &managed_worktrees {
        let branch = worktree.git_branch.as_deref().unwrap_or_default();
        if let Err(err) = ensure_worktree_attached(
            &workspace.root_path,
            root,
            &worktree.base_commit_sha,
            branch,
        )
        .await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to recreate worktree: {err:#}"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    for worktree in &worktrees {
        let sandbox_binding = store
            .get_sandbox_binding(worktree.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(binding) = sandbox_binding.as_ref() {
            let refreshed_binding =
                rematerialize_sandbox_binding_for_worktree(&state, &workspace, worktree, binding)
                    .await
                    .map_err(|err| {
                        tracing::warn!(
                            task_id = %task_id.0,
                            worktree_id = %worktree.id.0,
                            "failed to rematerialize sandbox worktree on unarchive: {err:#}"
                        );
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?;
            store
                .upsert_sandbox_binding(refreshed_binding)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Err(e) = crate::daemon::workspaces::attachments::ensure_worktree_attachment_mounts_if_materialized(
            &state,
            &workspace,
            worktree,
        )
        .await
        {
            tracing::warn!(task_id = %task_id.0, "attachment mounts failed: {e:?}");
        }
        if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
            Arc::clone(&state),
            workspace.clone(),
            worktree.clone(),
        )
        .await
        {
            tracing::warn!(task_id = %task_id.0, "worktree bootstrap failed: {e:?}");
        }
        if let Err(e) =
            vcs_hooks::ensure_task_commit_hook(&state, &workspace, worktree, task_id).await
        {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree.id.0,
                "failed to configure vcs hooks: {e:#}"
            );
        }
    }

    let updated = store
        .unarchive_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
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
        .emit_workspace_task_delta(task.clone(), TaskDeltaKind::Unarchived)
        .await;
    if let Err(e) = state.emit_workspace_task_upsert(task_id).await {
        tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    state
        .emit_workspace_archived_task_delete(task.workspace_id, task_id)
        .await;
    for session_id in session_ids {
        state
            .workspaces
            .workspace_active_snapshot
            .remove_session(session_id)
            .await;
        state.refresh_session_head_cache(session_id).await;
    }
    Ok(Json(task))
}
