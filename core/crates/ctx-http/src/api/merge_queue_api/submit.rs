use std::sync::Arc;

use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::{SessionId, WorktreeId};
use ctx_merge_queue::MergeQueueSubmitParams;

use super::request::MergeQueueSubmitReq;
use crate::api::errors::ApiErrorResp;
use crate::api::validate_scoped_mcp_session_context;
use crate::daemon::{merge_queue, AppState};

pub(in crate::api) async fn submit_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<ctx_mcp_auth::McpAuthContext>>,
    Json(req): Json<MergeQueueSubmitReq>,
) -> Result<Json<ctx_core::models::MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let mut session_id = match req.session_id {
        Some(id) => Some(SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid session_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let mut worktree_id = match req.worktree_id {
        Some(id) => Some(WorktreeId(uuid::Uuid::parse_str(&id).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree_id".to_string(),
                }),
            )
        })?)),
        None => None,
    };
    let worktree_root = req.worktree_root.and_then(|root| {
        let trimmed = root.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    if let Some(Extension(mcp_auth)) = mcp_auth {
        if worktree_root.is_some() {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error: "scoped ctx-mcp merge queue submit cannot override worktree_root"
                        .to_string(),
                }),
            ));
        }
        let scoped_session_id = session_id.unwrap_or(mcp_auth.session_id);
        let scoped_worktree_id = worktree_id.unwrap_or(mcp_auth.worktree_id);
        if !mcp_auth.allows_merge_queue_submit(scoped_session_id, scoped_worktree_id) {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResp {
                    error:
                        "scoped ctx-mcp merge queue submit is limited to the current session and worktree"
                            .to_string(),
                }),
            ));
        }
        validate_scoped_mcp_session_context(&state, mcp_auth, scoped_session_id).await?;
        session_id = Some(scoped_session_id);
        worktree_id = Some(scoped_worktree_id);
    }

    let params = MergeQueueSubmitParams {
        session_id,
        worktree_id,
        worktree_root,
        target_branch: req.target_branch,
        message: req.message,
    };
    let entry = merge_queue::submit_merge_queue_entry(&state, params)
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: err.to_string(),
                }),
            )
        })?;
    Ok(Json(entry))
}
