use super::*;

async fn create_session_for_task_inner(
    tasks: &TasksHandle,
    task_id: TaskId,
    headers: HeaderMap,
    req: CreateSessionReq,
) -> Result<Json<Session>, StatusCode> {
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let session = tasks
        .create_session_for_task(task_id, req.into_task_session_input(run_id_header))
        .await
        .map_err(task_session_create_status)?;
    Ok(Json(session))
}

pub(in crate::api) async fn create_session_for_task(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    create_session_for_task_inner(&tasks, task_id, headers, req).await
}
