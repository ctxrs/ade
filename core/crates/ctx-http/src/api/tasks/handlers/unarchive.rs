use super::super::*;

pub(in crate::api) async fn unarchive_task(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<Json<Task>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let task = tasks
        .unarchive_task(task_id)
        .await
        .map_err(task_lifecycle_status)?;
    Ok(Json(task))
}
