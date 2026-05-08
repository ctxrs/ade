use super::*;

pub(in crate::api) async fn get_workspace_active_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveSnapshot>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(workspace_hydration_status)?;
    crate::merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
    let snapshot = state
        .workspaces
        .workspace_active_snapshot
        .active_snapshot(workspace_id, i64::MAX)
        .await;
    state
        .cache_workspace_active_snapshot(snapshot.clone())
        .await;
    Ok(Json(snapshot))
}

pub(in crate::api) async fn get_workspace_active_heads(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<WorkspaceActiveHeadBatch>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    state
        .ensure_workspace_active_snapshot_hydrated(workspace_id)
        .await
        .map_err(workspace_hydration_status)?;
    crate::merge_queue::activate_workspace_merge_queue(&state, workspace_id).await;
    let heads = state
        .workspaces
        .workspace_active_snapshot
        .active_heads(workspace_id)
        .await;
    state.cache_workspace_active_heads(heads.clone()).await;
    Ok(Json(heads))
}

fn workspace_hydration_status(error: WorkspaceHydrationError) -> StatusCode {
    match error.kind() {
        WorkspaceHydrationErrorKind::NotFound => StatusCode::NOT_FOUND,
        WorkspaceHydrationErrorKind::Load => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
