use super::*;
use ctx_core::models::{ExecutionEnvironment, Session, Workspace, Worktree};
use ctx_settings_model::ExecutionSettings;
use ctx_store::Store;

pub(super) struct ParentWorktreeContext {
    pub(super) workspace: Workspace,
    pub(super) worktree: Worktree,
    pub(super) effective: ExecutionSettings,
    pub(super) execution_environment: ExecutionEnvironment,
}

pub(super) async fn validate_parent_spawn_capacity(
    store: &Store,
    parent: &Session,
    requested_count: usize,
) -> ApiResult<()> {
    if parent.parent_session_id.is_some() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "subagents cannot spawn child agents; max depth is {}",
                DEFAULT_MAX_SUBAGENT_DEPTH
            ),
        ));
    }
    let existing_active = store
        .count_active_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;
    if existing_active + requested_count > DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "max {} active child agents per parent",
                DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT
            ),
        ));
    }
    Ok(())
}

pub(super) async fn load_parent_worktree_context(
    state: &Arc<DaemonState>,
    store: &Store,
    parent: &Session,
) -> ApiResult<ParentWorktreeContext> {
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "workspace not found"))?;

    let parent_worktree_execution = crate::daemon::workspaces::resolve_existing_worktree_execution(
        state,
        store,
        &workspace,
        parent.worktree_id,
    )
    .await
    .map_err(internal_api_error)?;
    let execution_environment = parent_worktree_execution.execution_environment();
    if parent.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %parent.id.0,
            stored = parent.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "parent session execution_environment drifted from resolved worktree identity"
        );
    }

    Ok(ParentWorktreeContext {
        workspace,
        worktree: parent_worktree_execution.worktree,
        effective: parent_worktree_execution.effective,
        execution_environment,
    })
}
