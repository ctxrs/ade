use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_transport_runtime::web_sessions::{
    ensure_worker_bundle, validate_web_session_host_session, validate_web_session_host_worktree,
    validate_web_session_launch_scope, validate_web_session_url, NodeRuntimeSpec,
    WebSessionCreateRequest, WebSessionInfo, WebSessionLaunchPolicyError,
    WebSessionLaunchPolicyErrorKind, WebSessionViewport,
};

use crate::daemon::AppState;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_settings_model::ExecutionMode;
use ctx_settings_service::HostExecutionPolicy;

pub(crate) struct WebSessionLaunchRequest {
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) url: String,
    pub(crate) viewport: Option<WebSessionViewport>,
    pub(crate) fps: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebSessionLaunchErrorKind {
    BadRequest,
    Forbidden,
    Internal,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WebSessionLaunchError {
    kind: WebSessionLaunchErrorKind,
    message: String,
}

impl WebSessionLaunchError {
    pub(crate) fn kind(&self) -> WebSessionLaunchErrorKind {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

struct WebSessionLaunchContext {
    work_dir: Option<PathBuf>,
}

pub(crate) async fn create_web_session(
    state: &Arc<AppState>,
    request: WebSessionLaunchRequest,
) -> Result<WebSessionInfo, WebSessionLaunchError> {
    validate_web_session_url(&request.url).map_err(|e| bad_request(e.to_string()))?;

    let launch_context =
        resolve_web_session_launch_context(state, request.session_id, request.worktree_id)
            .await
            .map_err(request_or_policy_error)?;

    let node_runtime = crate::daemon::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        ctx_provider_install::install_state::InstallTarget::Host,
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare node runtime: {e}")))?;

    let worker_bundle = ensure_worker_bundle(
        &state.core.data_root,
        &NodeRuntimeSpec {
            node_bin: node_runtime.node_bin.clone(),
            npm_cli_js: node_runtime.npm_cli_js.clone(),
        },
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare web session worker: {e}")))?;

    let handle = state
        .transport
        .web_sessions
        .create(WebSessionCreateRequest {
            url: request.url,
            viewport: request.viewport,
            fps: request.fps,
            work_dir: launch_context.work_dir,
            session_id: request.session_id.map(|id| id.0.to_string()),
            worktree_id: request.worktree_id.map(|id| id.0.to_string()),
            node_bin: node_runtime.node_bin,
            worker_path: worker_bundle.worker_path,
            node_modules_path: worker_bundle.node_modules_path,
        })
        .await
        .map_err(|e| internal_error(format!("failed to create web session: {e}")))?;

    Ok(handle.snapshot().await)
}

async fn resolve_web_session_launch_context(
    state: &Arc<AppState>,
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
    state: &Arc<AppState>,
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

fn launch_error(
    kind: WebSessionLaunchErrorKind,
    message: impl Into<String>,
) -> WebSessionLaunchError {
    WebSessionLaunchError {
        kind,
        message: message.into(),
    }
}

fn bad_request(error: impl Into<String>) -> WebSessionLaunchError {
    launch_error(WebSessionLaunchErrorKind::BadRequest, error)
}

fn request_or_policy_error(error: anyhow::Error) -> WebSessionLaunchError {
    let kind = if let Some(policy_error) = error.downcast_ref::<WebSessionLaunchPolicyError>() {
        match policy_error.kind() {
            WebSessionLaunchPolicyErrorKind::BadRequest => WebSessionLaunchErrorKind::BadRequest,
            WebSessionLaunchPolicyErrorKind::Forbidden => WebSessionLaunchErrorKind::Forbidden,
        }
    } else if ctx_settings_service::is_execution_policy_denial(&error) {
        WebSessionLaunchErrorKind::Forbidden
    } else {
        WebSessionLaunchErrorKind::BadRequest
    };
    launch_error(kind, format!("{error:#}"))
}

fn internal_error(error: impl Into<String>) -> WebSessionLaunchError {
    launch_error(WebSessionLaunchErrorKind::Internal, error)
}

#[cfg(test)]
#[path = "launch/tests.rs"]
mod tests;
