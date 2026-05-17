use super::*;

pub(in crate::api) async fn create_workspace_attachment(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<CreateWorkspaceAttachmentRouteRequest>,
) -> Result<Json<Vec<WorkspaceAttachmentRouteResponse>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    workspaces
        .create_and_sync_workspace_attachment_for_route(workspace_id, req)
        .await
        .map_err(workspace_route_api_error)
        .map(Json)
}

pub(in crate::api) async fn delete_workspace_attachment(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
    Json(req): Json<DeleteWorkspaceAttachmentRouteRequest>,
) -> Result<Json<Vec<WorkspaceAttachmentRouteResponse>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    workspaces
        .delete_and_sync_workspace_attachment_for_route(workspace_id, req)
        .await
        .map_err(workspace_route_api_error)
        .map(Json)
}
