use super::*;

mod create;
mod delete;

pub(in crate::api) use create::create_workspace;
pub(in crate::api) use delete::delete_workspace;

pub(in crate::api) async fn list_workspaces(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Workspace>>, StatusCode> {
    state
        .global_store()
        .list_workspaces()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(in crate::api) async fn get_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Workspace>, StatusCode> {
    let id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    match state
        .global_store()
        .get_workspace(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        Some(ws) => {
            state
                .telemetry
                .telemetry
                .emit(TelemetryEvent::workspace_opened())
                .await;
            Ok(Json(ws))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}
