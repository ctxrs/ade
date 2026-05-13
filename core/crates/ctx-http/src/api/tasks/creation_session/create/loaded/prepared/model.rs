use super::*;
use crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment;

pub(super) struct ResolvedLoadedSessionModel {
    pub(super) model_id: String,
    pub(super) reasoning_effort: Option<String>,
    pub(super) preferred_model_id: String,
}

pub(super) struct LoadedSessionModelRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a Store,
    pub(super) workspace: &'a Workspace,
    pub(super) task_id: TaskId,
    pub(super) provider_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) requested_model_id: &'a str,
    pub(super) requested_reasoning_effort: Option<&'a str>,
    pub(super) created_worktree_id: Option<WorktreeId>,
}

pub(super) async fn resolve_loaded_session_model(
    request: LoadedSessionModelRequest<'_>,
) -> Result<ResolvedLoadedSessionModel, StatusCode> {
    let LoadedSessionModelRequest {
        state,
        store,
        workspace,
        task_id,
        provider_id,
        execution_environment,
        requested_model_id,
        requested_reasoning_effort,
        created_worktree_id,
    } = request;

    let catalog = match load_provider_model_catalog_for_execution_environment(
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
