use super::*;
use ctx_daemon::daemon::CreateWorkspaceRequest;

pub(in crate::api) async fn create_workspace(
    State(workspaces): State<WorkspacesHandle>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceRouteResponse>, (StatusCode, Json<ApiErrorResp>)> {
    workspaces
        .create_workspace_for_request(req)
        .await
        .map(Json)
        .map_err(workspace_route_api_error)
}
