use super::*;

pub(super) async fn submit_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Json(req): Json<MergeQueueSubmitReq>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = match req.session_id {
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
    let worktree_id = match req.worktree_id {
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
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let entries = store
        .list_merge_queue_entries(workspace_id, params.limit)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(entries))
}

pub(super) async fn cancel_merge_queue_entry(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::cancel_merge_queue_entry(&state, entry_id)
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
    Path(id): Path<String>,
) -> Result<Json<MergeQueueEntry>, (StatusCode, Json<ApiErrorResp>)> {
    let entry_id = MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid entry id".to_string(),
            }),
        )
    })?);
    let entry = merge_queue::retry_merge_queue_entry(&state, entry_id)
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
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let entry_id =
        MergeQueueEntryId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let entry = merge_queue::get_merge_queue_entry(&state, entry_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let store = state
        .store_for_workspace(entry.workspace_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let run = store
        .get_latest_merge_queue_run(entry_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let Some(path) = run.log_path.as_deref() else {
        return Err(StatusCode::NOT_FOUND);
    };
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
