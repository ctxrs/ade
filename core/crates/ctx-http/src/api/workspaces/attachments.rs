use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct SyncWorkspaceAttachmentsReq {
    #[serde(default)]
    refresh: Option<bool>,
}

pub(in crate::api) async fn list_workspace_attachments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkspaceAttachment>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_workspace_status(&state, ws_id).await?;
    store
        .list_workspace_attachments(ws_id)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(in crate::api) async fn sync_workspace_attachments(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SyncWorkspaceAttachmentsReq>,
) -> Result<Json<Vec<WorkspaceAttachment>>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let workspace = state
        .global_store()
        .get_workspace(ws_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let refresh = req.refresh.unwrap_or(false);
    let attachments = crate::daemon::workspaces::attachments::sync_workspace_attachments(
        Arc::clone(&state),
        &workspace,
        refresh,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let _ = crate::daemon::workspaces::attachments::ensure_workspace_attachments_for_worktrees_with_attachments(
        &state,
        &workspace,
        &attachments,
        false,
        false,
    )
    .await;
    Ok(Json(attachments))
}
