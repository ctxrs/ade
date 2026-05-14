use super::super::*;

#[derive(Debug, Serialize)]
pub(in crate::api) struct ArchiveTaskResponse {
    #[serde(flatten)]
    pub(in crate::api) task: Task,
    pub(in crate::api) cleanup_failed: bool,
}

pub(in crate::api) async fn archive_task(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<Json<ArchiveTaskResponse>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let outcome = tasks
        .archive_task(task_id)
        .await
        .map_err(task_lifecycle_status)?;
    Ok(Json(ArchiveTaskResponse {
        task: outcome.task,
        cleanup_failed: outcome.cleanup_failed,
    }))
}
