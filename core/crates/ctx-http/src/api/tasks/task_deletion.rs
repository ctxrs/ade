use super::*;

pub(in crate::api) async fn delete_task(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    tasks
        .delete_task(task_id)
        .await
        .map_err(task_lifecycle_status)?;
    Ok(StatusCode::NO_CONTENT)
}
