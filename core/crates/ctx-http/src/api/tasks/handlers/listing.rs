use super::super::*;

pub(in crate::api) async fn list_workspace_tasks(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Task>>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let tasks = tasks
        .list_workspace_tasks(workspace_id)
        .await
        .map_err(workspace_store_status)?;
    Ok(Json(tasks))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct WorkspaceArchivedQuery {
    limit: Option<u32>,
    cursor_sort_at: Option<String>,
    cursor_task_id: Option<String>,
}

pub(in crate::api) async fn list_workspace_archived_task_summaries(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
    Query(query): Query<WorkspaceArchivedQuery>,
) -> Result<Json<WorkspaceArchivedPage>, StatusCode> {
    let workspace_id =
        WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let limit = query.limit.unwrap_or(50) as i64;
    let cursor = match (
        query.cursor_sort_at.as_deref(),
        query.cursor_task_id.as_deref(),
    ) {
        (None, None) => None,
        (Some(sort_at), Some(task_id)) => {
            let sort_at = DateTime::parse_from_rfc3339(sort_at)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .with_timezone(&Utc);
            let task_id =
                TaskId(uuid::Uuid::parse_str(task_id).map_err(|_| StatusCode::BAD_REQUEST)?);
            Some(WorkspaceIndexCursor { sort_at, task_id })
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    let page = tasks
        .list_workspace_archived_page(workspace_id, cursor, limit)
        .await
        .map_err(workspace_store_status)?;
    Ok(Json(page))
}

pub(in crate::api) async fn list_task_sessions(
    State(tasks): State<TasksHandle>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Session>>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sessions = tasks
        .list_task_sessions(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(sessions))
}

fn workspace_store_status(error: ctx_daemon::daemon::WorkspaceStoreAccessError) -> StatusCode {
    match error {
        ctx_daemon::daemon::WorkspaceStoreAccessError::NotFound => StatusCode::NOT_FOUND,
        ctx_daemon::daemon::WorkspaceStoreAccessError::Unavailable(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
