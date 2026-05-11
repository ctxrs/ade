use super::*;

pub(super) struct ResolvedLoadedSessionModel {
    pub(super) model_id: String,
    pub(super) reasoning_effort: Option<String>,
    pub(super) preferred_model_id: String,
}

pub(super) async fn resolve_loaded_session_model(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task_id: TaskId,
    provider_id: &str,
    execution_environment: ExecutionEnvironment,
    requested_model_id: &str,
    requested_reasoning_effort: Option<&str>,
    created_worktree_id: Option<WorktreeId>,
) -> Result<ResolvedLoadedSessionModel, StatusCode> {
    let catalog = match sessions::load_provider_model_catalog_for_execution_environment(
        state,
        workspace,
        provider_id,
        execution_environment,
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(error) => {
            tracing::warn!(
                workspace_id = %workspace.id.0,
                provider_id = provider_id,
                execution_environment = execution_environment.as_str(),
                "failed to load provider model catalog while creating session: {error}"
            );
            cleanup_created_worktree(state, store, workspace, task_id, created_worktree_id).await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let resolved_model = match resolve_model_id(
        Some(requested_model_id),
        requested_reasoning_effort,
        None,
        catalog.as_ref(),
    ) {
        Ok(model) => model,
        Err(_) => {
            cleanup_created_worktree(state, store, workspace, task_id, created_worktree_id).await;
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let model_id = resolved_model.model_id.clone();
    let reasoning_effort = resolved_model.reasoning_effort.clone();
    let preferred_model_id = compose_model_id(&model_id, reasoning_effort.as_deref());

    Ok(ResolvedLoadedSessionModel {
        model_id,
        reasoning_effort,
        preferred_model_id,
    })
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
