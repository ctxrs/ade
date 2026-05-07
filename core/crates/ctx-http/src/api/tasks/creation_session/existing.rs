use super::*;

pub(super) async fn resolve_existing_requested_session(
    state: &Arc<AppState>,
    store: &Store,
    task: &Task,
    workspace: &Workspace,
    requested_session_id: Option<SessionId>,
    created_worktree_id: Option<WorktreeId>,
    worktree_id: WorktreeId,
    execution_environment: ExecutionEnvironment,
    provider_id: &str,
    model_id: &str,
    reasoning_effort: Option<&str>,
    parent_session_id: Option<SessionId>,
    relationship: Option<&str>,
    remember_model_preference: bool,
    preferred_model_id: &str,
) -> Result<Option<Session>, StatusCode> {
    let Some(session_id) = requested_session_id else {
        return Ok(None);
    };
    let existing_ws = state
        .global_store()
        .get_workspace_id_for_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some(existing_ws) = existing_ws else {
        return Ok(None);
    };
    if existing_ws != task.workspace_id {
        cleanup_created_worktree(state, store, workspace, task.id, created_worktree_id).await;
        return Err(StatusCode::CONFLICT);
    }

    let existing = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some(existing) = existing else {
        cleanup_created_worktree(state, store, workspace, task.id, created_worktree_id).await;
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    if !session_matches_creation_identity(
        &existing,
        SessionCreationIdentity {
            task_id: task.id,
            workspace_id: task.workspace_id,
            worktree_id,
            execution_environment,
            provider_id,
            model_id,
            reasoning_effort,
            parent_session_id,
            relationship,
        },
    ) {
        cleanup_created_worktree(state, store, workspace, task.id, created_worktree_id).await;
        return Err(StatusCode::CONFLICT);
    }

    state.sessions.remember_session_meta(&existing).await;
    if remember_model_preference {
        if let Err(error) =
            crate::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
                state,
                task.workspace_id,
                provider_id,
                Some(preferred_model_id.to_string()),
            )
            .await
        {
            tracing::warn!(
                session_id = %existing.id.0,
                workspace_id = %task.workspace_id.0,
                provider_id = provider_id,
                "failed to persist workspace provider model preference: {error:#}"
            );
        }
    }

    Ok(Some(existing))
}

async fn cleanup_created_worktree(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task_id: TaskId,
    created_worktree_id: Option<WorktreeId>,
) {
    if let Some(created_worktree_id) = created_worktree_id {
        cleanup_orphaned_provisioned_worktree(
            state,
            store,
            workspace,
            task_id,
            created_worktree_id,
        )
        .await;
    }
}
