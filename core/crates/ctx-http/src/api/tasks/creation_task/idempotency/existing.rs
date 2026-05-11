use super::identity::{task_id_conflict, validate_requested_task_identity};
use super::*;

pub(in crate::api::tasks::creation::task_creation) async fn load_existing_task_for_request(
    state: &Arc<AppState>,
    store: &Store,
    ws_id: WorkspaceId,
    request: &CreateTaskRequestParts,
) -> Result<Option<Task>, CreateTaskApiError> {
    let Some(task_id) = request.task_id else {
        return Ok(None);
    };
    let existing_ws = state
        .global_store()
        .get_workspace_id_for_task(task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let Some(existing_ws) = existing_ws else {
        return Ok(None);
    };
    if existing_ws != ws_id {
        return Err(task_id_conflict());
    }
    let existing = store.get_task(task_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let Some(existing) = existing else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "task index exists but task missing".to_string(),
            }),
        ));
    };
    validate_requested_task_identity(&existing, ws_id, request)?;
    Ok(Some(existing))
}
