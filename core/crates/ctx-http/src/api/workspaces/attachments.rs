use super::*;
use ctx_core::ids::WorkspaceId;

pub(in crate::api) async fn list_workspace_attachments(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<WorkspaceAttachmentRouteResponse>>, StatusCode> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    workspaces
        .list_workspace_attachments_for_route(ws_id)
        .await
        .map(Json)
        .map_err(|error| workspace_route_status(&error))
}

pub(in crate::api) async fn sync_workspace_attachments(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<SyncWorkspaceAttachmentsRouteRequest>,
) -> Result<Json<Vec<WorkspaceAttachmentRouteResponse>>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = parse_workspace_id(&id)?;
    workspaces
        .sync_workspace_attachments_for_route(ws_id, req)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}
