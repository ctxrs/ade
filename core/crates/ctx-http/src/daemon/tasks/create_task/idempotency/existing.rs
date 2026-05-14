use super::identity::{task_id_conflict, validate_requested_task_identity};
use super::*;

pub(in crate::daemon::tasks::create_task) async fn load_existing_task_for_request(
    handles: &TaskCreationHandles,
    store: &Store,
    ws_id: WorkspaceId,
    request: &CreateTaskRequestParts,
) -> Result<Option<Task>, CreateTaskApiError> {
    let Some(task_id) = request.task_id else {
        return Ok(None);
    };
    let existing_ws = handles
        .sessions
        .get_workspace_id_for_task(task_id)
        .await
        .map_err(TaskCreateError::internal)?;
    let Some(existing_ws) = existing_ws else {
        return Ok(None);
    };
    if existing_ws != ws_id {
        return Err(task_id_conflict());
    }
    let existing = store
        .get_task(task_id)
        .await
        .map_err(TaskCreateError::internal)?;
    let Some(existing) = existing else {
        return Err(TaskCreateError::Internal(anyhow::anyhow!(
            "task index exists but task missing"
        )));
    };
    validate_requested_task_identity(&existing, ws_id, request)?;
    Ok(Some(existing))
}
