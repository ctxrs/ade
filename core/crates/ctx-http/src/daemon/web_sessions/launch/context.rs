use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_settings_model::ExecutionMode;
use ctx_settings_service::HostExecutionPolicy;
use ctx_transport_runtime::web_sessions::{
    validate_web_session_host_session, validate_web_session_host_worktree,
    validate_web_session_launch_scope,
};

use crate::daemon::DaemonState;

pub(super) struct WebSessionLaunchContext {
    pub(super) work_dir: Option<PathBuf>,
}

pub(super) async fn resolve_web_session_launch_context(
    state: &Arc<DaemonState>,
    session_id: Option<SessionId>,
    worktree_id: Option<WorktreeId>,
) -> anyhow::Result<WebSessionLaunchContext> {
    HostExecutionPolicy::current()?
        .validate_execution_environment(ExecutionEnvironment::Host)
        .context("web sessions currently run on the host")?;

    validate_web_session_launch_scope(session_id.is_some(), worktree_id.is_some())?;

    let mut session_worktree_id = None;
    if let Some(session_id) = session_id {
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        validate_web_session_host_session(session.execution_environment)?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        validate_web_session_worktree(state, &store, &worktree).await?;
        session_worktree_id = Some(session.worktree_id);
    }

    if let Some(worktree_id) = worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        validate_web_session_worktree(state, &store, &worktree).await?;
        return Ok(WebSessionLaunchContext {
            work_dir: Some(PathBuf::from(worktree.root_path)),
        });
    }

    if let Some(worktree_id) = session_worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(WebSessionLaunchContext {
            work_dir: Some(PathBuf::from(worktree.root_path)),
        });
    }
    Ok(WebSessionLaunchContext { work_dir: None })
}

async fn validate_web_session_worktree(
    state: &Arc<DaemonState>,
    store: &ctx_store::Store,
    worktree: &Worktree,
) -> anyhow::Result<()> {
    let has_sandbox_binding = store.get_sandbox_binding(worktree.id).await?.is_some();
    let effective = crate::daemon::execution_effective::effective_execution_settings(
        state,
        worktree.workspace_id,
    )
    .await
    .context("loading workspace execution settings for web session")?;
    validate_web_session_host_worktree(
        has_sandbox_binding,
        matches!(effective.mode, ExecutionMode::Sandbox),
    )?;
    Ok(())
}
