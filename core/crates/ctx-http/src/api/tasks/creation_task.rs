use super::*;

type CreateTaskApiError = (StatusCode, Json<ApiErrorResp>);

pub(in crate::api) async fn create_task(
    State(tasks): State<TasksHandle>,
    State(_sessions): State<SessionsHandle>,
    State(_providers): State<ProvidersHandle>,
    State(_workspaces): State<WorkspacesHandle>,
    State(_transport): State<TransportHandle>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Json<Task>, CreateTaskApiError> {
    let workspace_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let task = tasks
        .create_task_for_workspace(workspace_id, req.into_create_task_input()?)
        .await
        .map_err(task_create_api_error)?;
    Ok(Json(task))
}
