use super::*;

pub(super) async fn load_create_task_workspace(
    handles: &TaskApiHandles,
    id: &str,
) -> Result<(WorkspaceId, Workspace, Store), CreateTaskApiError> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let ctx = handles
        .sessions
        .load_workspace_context(ws_id)
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
    Ok((ctx.workspace_id, ctx.workspace, ctx.store))
}
