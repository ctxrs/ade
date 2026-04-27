use super::*;
use crate::api::shared::{path_resolves_within_root, store_for_existing_workspace_status};

pub(super) async fn submit_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Json(req): Json<MergeQueueSubmitReq>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
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
        session_id = Some(scoped_session_id);
        worktree_id = Some(scoped_worktree_id);
    }

    let params = merge_queue::MergeQueueSubmitParams {
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

pub(super) async fn list_merge_queue_entries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<MergeQueueListParams>,
) -> Result<Json<Vec<MergeQueueEntry>>, StatusCode> {
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&params.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let entries = store
        .list_merge_queue_entries(workspace_id, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(entries))
}

pub(super) async fn cancel_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&workspace_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::cancel_merge_queue_entry(&state, workspace_id, entry_id)
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

pub(super) async fn retry_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&workspace_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::retry_merge_queue_entry(&state, workspace_id, entry_id)
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

pub(super) async fn get_merge_queue_entry_logs(
    State(state): State<Arc<AppState>>,
    Path((workspace_id, id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let entry_id =
        MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    merge_queue::get_workspace_merge_queue_entry(&state, workspace_id, entry_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let workspace = store
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let run = store
        .get_latest_merge_queue_run(entry_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = run.log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let log_root = std::path::PathBuf::from(&workspace.root_path)
        .join(".ctx")
        .join("merge-queue")
        .join("logs");
    if !path_resolves_within_root(std::path::Path::new(path), &log_root).await {
        return Err(StatusCode::NOT_FOUND);
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let filename = format!("merge-queue-{}.log", entry_id.0);
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| header::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}
