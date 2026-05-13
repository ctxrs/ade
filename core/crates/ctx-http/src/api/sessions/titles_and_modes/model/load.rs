use ctx_core::models::{ExecutionEnvironment, Workspace};
use ctx_provider_install::install_state::InstallTarget;
use ctx_store::Store;

use super::*;
use crate::api::sessions::titles_and_modes::model::error::{
    internal_session_model_error, session_model_error, SessionModelResult,
};

pub(super) struct SessionModelTarget {
    pub(super) store: Store,
    pub(super) session: Session,
    pub(super) workspace: Workspace,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) install_target: InstallTarget,
}

pub(super) async fn load_session_model_target(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> SessionModelResult<SessionModelTarget> {
    let store = store_for_existing_session_api_error_for_write(state, session_id)
        .await
        .map_err(|(status, resp)| session_model_error(status, resp.0.error))?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(internal_session_model_error)?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "session not found"))?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(internal_session_model_error)?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "workspace not found"))?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(internal_session_model_error)?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "worktree not found"))?;
    let resolved_worktree = crate::daemon::workspaces::resolve_existing_worktree_execution(
        state,
        &store,
        &workspace,
        worktree.id,
    )
    .await
    .map_err(internal_session_model_error)?;
    let execution_environment = resolved_worktree.execution_environment();
    if session.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %session.id.0,
            stored = session.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "session model update resolved a different execution_environment than persisted metadata"
        );
    }
    let install_target = execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        execution_environment,
    )
    .await
    .map_err(|err| {
        tracing::warn!(
            workspace_id = %worktree.workspace_id.0,
            "set_session_model failed to load execution settings: {err:#}",
        );
        session_model_error(
            crate::api::shared::status_code_for_internal_error(&err),
            "failed to load execution settings",
        )
    })?;

    Ok(SessionModelTarget {
        store,
        session,
        workspace,
        execution_environment,
        install_target,
    })
}
