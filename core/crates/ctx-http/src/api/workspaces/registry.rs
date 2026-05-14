use super::*;

mod create;
mod delete;

pub(in crate::api) use create::create_workspace;
pub(in crate::api) use delete::delete_workspace;

pub(in crate::api) async fn list_workspaces(
    State(workspaces): State<WorkspacesHandle>,
) -> Result<Json<Vec<Workspace>>, StatusCode> {
    workspaces
        .list_workspaces()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(in crate::api) async fn get_workspace(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match workspaces
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(ws) => {
            workspaces.record_workspace_opened().await;
            Ok(Json(ws))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}
