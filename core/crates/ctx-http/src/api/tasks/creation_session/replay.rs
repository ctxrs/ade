use super::*;

pub(in crate::api::tasks) async fn create_requested_default_session_for_task(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    req: CreateTaskDefaultSessionReq,
) -> Result<Session, StatusCode> {
    let Json(session) = create_session_for_loaded_task_inner(
        state,
        store,
        task,
        workspace,
        HeaderMap::new(),
        req.into_create_session_req(),
    )
    .await?;
    Ok(session)
}

pub(in crate::api::tasks) async fn replay_requested_default_session_for_task(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    req: CreateTaskDefaultSessionReq,
    primary_session_id: SessionId,
) -> Result<Session, StatusCode> {
    let Json(session) = create_session_for_loaded_task_inner(
        state,
        store,
        task,
        workspace,
        HeaderMap::new(),
        req.into_replay_create_session_req(primary_session_id),
    )
    .await?;
    Ok(session)
}
