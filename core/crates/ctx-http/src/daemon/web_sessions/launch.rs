use std::sync::Arc;

mod context;

use context::resolve_web_session_launch_context;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_transport_runtime::web_sessions::{
    validate_web_session_url, WebSessionCreateRequest, WebSessionInfo, WebSessionLaunchPolicyError,
    WebSessionLaunchPolicyErrorKind, WebSessionViewport,
};

use crate::daemon::web_sessions::prepare_web_session_worker;
use crate::daemon::AppState;

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

pub(crate) async fn create_web_session(
    state: &Arc<AppState>,
    request: WebSessionLaunchRequest,
) -> Result<WebSessionInfo, WebSessionLaunchError> {
    validate_web_session_url(&request.url).map_err(|e| bad_request(e.to_string()))?;

    let launch_context =
        resolve_web_session_launch_context(state, request.session_id, request.worktree_id)
            .await
            .map_err(request_or_policy_error)?;

    let worker = prepare_web_session_worker(state)
        .await
        .map_err(|error| internal_error(format!("{error:#}")))?;

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
            node_bin: worker.node_runtime.node_bin,
            worker_path: worker.bundle.worker_path,
            node_modules_path: worker.bundle.node_modules_path,
        })
        .await
        .map_err(|e| internal_error(format!("failed to create web session: {e}")))?;

    Ok(handle.snapshot().await)
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
