use super::*;

pub(super) fn validate_requested_task_identity(
    task: &Task,
    ws_id: WorkspaceId,
    request: &CreateTaskRequestParts,
) -> Result<(), CreateTaskApiError> {
    if request.task_id.is_none() {
        return Ok(());
    }
    if task.workspace_id != ws_id
        || !task_request_matches(task, &request.title, &request.description)
    {
        return Err(task_id_conflict());
    }
    Ok(())
}

pub(super) fn internal_store_error(error: impl std::fmt::Display) -> CreateTaskApiError {
    TaskCreateError::Internal(anyhow::anyhow!(logs::redact_sensitive(&error.to_string())))
}

pub(super) fn task_id_conflict() -> CreateTaskApiError {
    TaskCreateError::Conflict("task id already exists".to_string())
}

pub(super) fn task_not_found() -> CreateTaskApiError {
    TaskCreateError::NotFound("task not found".to_string())
}
