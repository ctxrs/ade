use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::web_sessions::{WebSessionCreateRequest, WebSessionInfo, WebSessionViewport};
use ctx_core::ids::{SessionId, WorktreeId};

pub(crate) struct WebSessionLaunchRequest {
    pub(crate) session_id: Option<SessionId>,
    pub(crate) worktree_id: Option<WorktreeId>,
    pub(crate) url: String,
    pub(crate) viewport: Option<WebSessionViewport>,
    pub(crate) fps: Option<u32>,
}

pub(crate) async fn create_web_session(
    state: &Arc<AppState>,
    request: WebSessionLaunchRequest,
) -> Result<WebSessionInfo, (StatusCode, Json<ApiErrorResp>)> {
    let work_dir = resolve_web_session_work_dir(state, request.session_id, request.worktree_id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let node_runtime = crate::installer::ensure_node_runtime(
        state.as_ref(),
        None,
        "web_session_worker",
        &state.core.data_root,
        ctx_provider_install::install_state::InstallTarget::Host,
    )
    .await
    .map_err(|e| internal_error(format!("failed to prepare node runtime: {e}")))?;

    let worker_bundle = crate::web_sessions::ensure_worker_bundle_for_node_runtime(
        &state.core.data_root,
        &node_runtime,
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
            work_dir,
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

pub(crate) async fn resolve_web_session_work_dir(
    state: &Arc<AppState>,
    session_id: Option<SessionId>,
    worktree_id: Option<WorktreeId>,
) -> anyhow::Result<Option<PathBuf>> {
    if let Some(worktree_id) = worktree_id {
        let store = state.store_for_worktree(worktree_id).await?;
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    if let Some(session_id) = session_id {
        let store = state.store_for_session(session_id).await?;
        let session = store
            .get_session(session_id)
            .await?
            .context("session not found")?;
        let worktree = store
            .get_worktree(session.worktree_id)
            .await?
            .context("worktree not found")?;
        return Ok(Some(PathBuf::from(worktree.root_path)));
    }
    Ok(None)
}

fn error_response(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn bad_request(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::BAD_REQUEST, error)
}

fn internal_error(error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    error_response(StatusCode::INTERNAL_SERVER_ERROR, error)
}
