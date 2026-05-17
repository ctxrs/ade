use super::*;
use ctx_core::ids::WorkspaceId;

pub(in crate::api) async fn get_workspace_active_snapshot(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveSnapshotRouteResponse>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let snapshot = workspaces
        .load_workspace_active_snapshot_for_route(workspace_id)
        .await
        .map_err(workspace_hydration_status)?;
    Ok(Json(snapshot))
}

pub(in crate::api) async fn get_workspace_active_heads(
    State(workspaces): State<WorkspacesHandle>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveHeadBatchRouteResponse>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let heads = workspaces
        .load_workspace_active_heads_for_route(workspace_id)
        .await
        .map_err(workspace_hydration_status)?;
    Ok(Json(heads))
}

fn workspace_hydration_status(error: WorkspaceHydrationError) -> StatusCode {
    match error.kind() {
        WorkspaceHydrationErrorKind::NotFound => StatusCode::NOT_FOUND,
        WorkspaceHydrationErrorKind::Load => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
