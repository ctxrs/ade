use super::*;
use ctx_core::ids::WorkspaceId;

mod create;
mod delete;

pub(in crate::api) use create::create_workspace;
pub(in crate::api) use delete::delete_workspace;

pub(in crate::api) async fn list_workspaces(
    State(workspaces): State<WorkspacesHandle>,
) -> Result<Json<Vec<WorkspaceRouteResponse>>, StatusCode> {
    workspaces
        .list_workspaces_for_route()
        .await
        .map(Json)
        .map_err(|error| workspace_route_status(&error))
}

pub(in crate::api) async fn get_workspace(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceRouteResponse>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match workspaces.get_workspace_for_route(id).await {
        Ok(Some(ws)) => Ok(Json(ws)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(error) => Err(workspace_route_status(&error)),
    }
}
