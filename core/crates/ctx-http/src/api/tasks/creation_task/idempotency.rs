use super::*;

#[path = "idempotency/existing.rs"]
mod existing;
#[path = "idempotency/identity.rs"]
mod identity;
#[path = "idempotency/request.rs"]
mod request;

pub(super) use self::request::CreateTaskRequestParts;
pub(super) use existing::load_existing_task_for_request;
use identity::{internal_store_error, task_not_found, validate_requested_task_identity};

pub(super) struct PersistedTaskForCreate {
    pub(super) task: Task,
    pub(super) created_in_this_request: bool,
}

pub(super) async fn persist_task_for_request(
    store: &Store,
    ws_id: WorkspaceId,
    existing_task: Option<Task>,
    request: &CreateTaskRequestParts,
) -> Result<PersistedTaskForCreate, CreateTaskApiError> {
    let (task, created_in_this_request) = match existing_task {
        Some(existing) => (existing, false),
        None => match request.task_id {
            Some(task_id) => {
                let result = store
                    .create_task_with_id_result(
                        ws_id,
                        task_id,
                        request.requested_title.clone(),
                        request.requested_description.clone(),
                    )
                    .await
                    .map_err(internal_store_error)?;
                (result.task, result.created)
            }
            None => (
                store
                    .create_task(
                        ws_id,
                        request.requested_title.clone(),
                        request.requested_description.clone(),
                    )
                    .await
                    .map_err(internal_store_error)?,
                true,
            ),
        },
    };
    validate_requested_task_identity(&task, ws_id, request)?;
    Ok(PersistedTaskForCreate {
        task,
        created_in_this_request,
    })
}

pub(super) async fn upsert_workspace_task_index(
    state: &Arc<AppState>,
    task_id: TaskId,
    ws_id: WorkspaceId,
) {
    if let Err(e) = state
        .global_store()
        .upsert_workspace_task_index(task_id, ws_id)
        .await
    {
        tracing::warn!(task_id = %task_id.0, "failed to update task index: {e:?}");
    }
}

pub(super) async fn reload_or_retry_task_for_request(
    store: &Store,
    ws_id: WorkspaceId,
    persisted: PersistedTaskForCreate,
    request: &CreateTaskRequestParts,
) -> Result<PersistedTaskForCreate, CreateTaskApiError> {
    let task = store
        .get_task_with_activity(persisted.task.id)
        .await
        .map_err(internal_store_error)?;
    if let Some(task) = task {
        return Ok(PersistedTaskForCreate {
            task,
            created_in_this_request: persisted.created_in_this_request,
        });
    }
    if persisted.created_in_this_request {
        return Err(task_not_found());
    }
    let Some(task_id) = request.task_id else {
        return Err(task_not_found());
    };
    let retry = store
        .create_task_with_id_result(
            ws_id,
            task_id,
            request.requested_title.clone(),
            request.requested_description.clone(),
        )
        .await
        .map_err(internal_store_error)?;
    if !retry.created {
        validate_requested_task_identity(&retry.task, ws_id, request)?;
    }
    Ok(PersistedTaskForCreate {
        task: retry.task,
        created_in_this_request: retry.created,
    })
}
