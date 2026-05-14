use super::*;

pub(in crate::api::tasks) async fn create_requested_default_session_for_task(
    handles: &TaskApiHandles,
    store: Store,
    task: Task,
    workspace: Workspace,
    req: CreateTaskDefaultSessionReq,
) -> Result<Session, StatusCode> {
    handles
        .tasks
        .create_session_for_loaded_task(
            store,
            task.clone(),
            workspace,
            req.into_task_session_input(None),
        )
        .await
        .map_err(task_session_create_status)
}

pub(in crate::api::tasks) async fn replay_requested_default_session_for_task(
    handles: &TaskApiHandles,
    store: Store,
    task: Task,
    workspace: Workspace,
    req: CreateTaskDefaultSessionReq,
    primary_session_id: SessionId,
) -> Result<Session, StatusCode> {
    handles
        .tasks
        .create_session_for_loaded_task(
            store,
            task.clone(),
            workspace,
            req.into_replay_task_session_input(primary_session_id),
        )
        .await
        .map_err(task_session_create_status)
}
