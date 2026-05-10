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
        || !task_request_matches(
            task,
            &request.requested_title,
            &request.requested_description,
        )
    {
        return Err(task_id_conflict());
    }
    Ok(())
}

pub(super) fn internal_store_error(error: impl std::fmt::Display) -> CreateTaskApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(&error.to_string()),
        }),
    )
}

pub(super) fn task_id_conflict() -> CreateTaskApiError {
    (
        StatusCode::CONFLICT,
        Json(ApiErrorResp {
            error: "task id already exists".to_string(),
        }),
    )
}

pub(super) fn task_not_found() -> CreateTaskApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResp {
            error: "task not found".to_string(),
        }),
    )
}
