use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct SyncWorkspaceAttachmentsReq {
    #[serde(default)]
    refresh: Option<bool>,
}

pub(in crate::api) async fn list_workspace_attachments(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkspaceAttachment>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    workspaces
        .list_workspace_attachments(ws_id)
        .await
        .map(Json)
        .map_err(|error| match error {
            ctx_daemon::daemon::WorkspaceStoreAccessError::NotFound => StatusCode::NOT_FOUND,
            ctx_daemon::daemon::WorkspaceStoreAccessError::Unavailable(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })
}

pub(in crate::api) async fn sync_workspace_attachments(
    State(workspaces): State<WorkspacesHandle>,
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
    let workspace = workspaces
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
    let attachments = workspaces
        .sync_workspace_attachments(&workspace, refresh)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    Ok(Json(attachments))
}
