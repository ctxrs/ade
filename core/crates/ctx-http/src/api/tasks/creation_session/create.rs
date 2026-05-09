use super::*;

#[path = "create/loaded.rs"]
mod loaded;
#[path = "create/persistence.rs"]
mod persistence;

pub(in crate::api::tasks) use loaded::create_session_for_loaded_task_inner;

async fn create_session_for_task_inner(
    state: Arc<AppState>,
    task_id: TaskId,
    headers: HeaderMap,
    req: CreateSessionReq,
) -> Result<Json<Session>, StatusCode> {
    let store = state
        .store_for_task(task_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let task = store
        .get_task(task_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .global_store()
        .get_workspace(task.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    create_session_for_loaded_task_inner(state, store, task, workspace, headers, req).await
}

pub(in crate::api) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let creation_lock = state.sessions.task_session_creation_lock(task_id).await;
    let _creation_guard = creation_lock.lock().await;
    create_session_for_task_inner(state, task_id, headers, req).await
}

pub(in crate::api) struct DefaultSessionSeed {
    pub provider_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub execution_environment: ExecutionEnvironment,
}

pub(in crate::api) async fn create_default_session_for_task(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    seed: DefaultSessionSeed,
) -> Result<Session, StatusCode> {
    let Json(session) = create_session_for_loaded_task_inner(
        state,
        store,
        task,
        workspace,
        HeaderMap::new(),
        CreateSessionReq {
            id: None,
            provider_id: seed.provider_id,
            model_id: seed.model_id,
            reasoning_effort: seed.reasoning_effort,
            remember_model_preference: false,
            parent_session_id: None,
            relationship: None,
            initial_prompt: None,
            initial_message_id: None,
            initial_turn_id: None,
            worktree_id: None,
            execution_environment: Some(seed.execution_environment),
        },
    )
    .await?;
    Ok(session)
}
