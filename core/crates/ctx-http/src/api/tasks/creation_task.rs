use super::*;

#[path = "creation_task/default_session_flow.rs"]
mod default_session_flow;
#[path = "creation_task/default_session_plan.rs"]
mod default_session_plan;
#[path = "creation_task/idempotency.rs"]
mod idempotency;
#[path = "creation_task/workspace.rs"]
mod workspace;
use default_session_flow::ensure_default_session_for_task;
use default_session_plan::preflight_default_session_creation;
use idempotency::{
    load_existing_task_for_request, persist_task_for_request, reload_or_retry_task_for_request,
    upsert_workspace_task_index, CreateTaskRequestParts,
};
use workspace::load_create_task_workspace;

type CreateTaskApiError = (StatusCode, Json<ApiErrorResp>);

pub(in crate::api) async fn create_task(
    State(sessions): State<SessionsHandle>,
    State(providers): State<ProvidersHandle>,
    State(workspaces): State<WorkspacesHandle>,
    State(_transport): State<TransportHandle>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Json<Task>, CreateTaskApiError> {
    let handles = TaskApiHandles::new(sessions, providers, workspaces);
    let (ws_id, ws, store) = load_create_task_workspace(&handles, &id).await?;
    let request = CreateTaskRequestParts::from_request(req)?;
    let existing_task = load_existing_task_for_request(&handles, &store, ws_id, &request).await?;
    let default_session_plan = if request.should_preflight_default_session(&existing_task) {
        Some(preflight_default_session_creation(&handles, &store, &ws).await?)
    } else {
        None
    };
    let persisted_task = persist_task_for_request(&store, ws_id, existing_task, &request).await?;
    upsert_workspace_task_index(&handles, persisted_task.task.id, ws_id).await;

    let default_session_lock = handles
        .sessions
        .task_session_creation_lock(persisted_task.task.id)
        .await;
    let _default_session_guard = default_session_lock.lock().await;
    let persisted_task =
        reload_or_retry_task_for_request(&store, ws_id, persisted_task, &request).await?;
    let task = ensure_default_session_for_task(
        &handles,
        store.clone(),
        ws,
        persisted_task.task,
        request.requested_default_session,
        default_session_plan,
        persisted_task.created_in_this_request,
    )
    .await?;
    Ok(Json(task))
}
