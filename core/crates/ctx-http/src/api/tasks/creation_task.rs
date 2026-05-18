use super::*;

type CreateTaskApiError = (StatusCode, Json<ApiErrorResp>);

pub(in crate::api) async fn create_task(
    State(tasks): State<TasksHandle>,
    State(_sessions): State<SessionsHandle>,
    State(_providers): State<ProvidersHandle>,
    State(_workspaces): State<WorkspacesHandle>,
    State(_transport): State<TransportHandle>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskRouteRequest>,
) -> Result<Json<TaskRouteResponse>, CreateTaskApiError> {
    let task = tasks
        .create_task_for_route(&id, req)
        .await
        .map_err(task_route_api_error)?;
    Ok(Json(task))
}
