use super::*;

pub(super) struct PreparedLoadedSessionRequest {
    pub(super) run_id_header: Option<String>,
    pub(super) provider_id: String,
    pub(super) session_id: Option<SessionId>,
    pub(super) parent_session_id: Option<SessionId>,
    pub(super) relationship: Option<String>,
    pub(super) requested_relationship: Option<String>,
    pub(super) worktree_id: WorktreeId,
    pub(super) created_worktree_id: Option<WorktreeId>,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) model_id: String,
    pub(super) reasoning_effort: Option<String>,
    pub(super) preferred_model_id: String,
}

pub(super) async fn prepare_loaded_session_request(
    state: &Arc<AppState>,
    store: &Store,
    task: &Task,
    workspace: &Workspace,
    headers: &HeaderMap,
    req: &CreateSessionReq,
) -> Result<PreparedLoadedSessionRequest, StatusCode> {
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let provider_id = req.provider_id.trim().to_string();
    if !state
        .providers
        .adapters
        .lock()
        .await
        .contains_key(&provider_id)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_request = match validate_create_session_request(CreateSessionRequestPolicy {
        requested_session_id: req.id.as_deref(),
        parent_session_id: req.parent_session_id.as_deref(),
        relationship: req.relationship.as_deref(),
        initial_prompt_present: req.initial_prompt.is_some(),
        initial_message_id_present: req.initial_message_id.is_some(),
        initial_turn_id_present: req.initial_turn_id.is_some(),
        task_primary_session_id: task.primary_session_id,
    }) {
        Ok(decision) => decision,
        Err(CreateSessionRequestError::MissingInitialPromptIds) => {
            state
                .emit_compat_payload_reject_counter(
                    "tasks.create_session",
                    "missing_initial_ids",
                    None,
                )
                .await;
            return Err(StatusCode::BAD_REQUEST);
        }
        Err(CreateSessionRequestError::PrimarySessionConflict) => {
            return Err(StatusCode::CONFLICT);
        }
        Err(
            CreateSessionRequestError::InvalidSessionId
            | CreateSessionRequestError::InvalidParentSessionId
            | CreateSessionRequestError::RelationshipRequiresParent,
        ) => {
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let session_id = session_request.session_id;
    let parent_session_id = session_request.parent_session_id;
    let relationship = session_request.relationship;
    let requested_relationship = relationship.clone();

    let worktree_resolution = resolve_session_worktree_for_task(
        state,
        store,
        task,
        workspace,
        req.worktree_id.as_deref(),
        req.execution_environment,
    )
    .await?;
    let worktree_id = worktree_resolution.worktree_id;
    let created_worktree_id = worktree_resolution.created_worktree_id;
    let execution_environment = worktree_resolution.execution_environment;
    let catalog = match sessions::load_provider_model_catalog_for_execution_environment(
        state,
        workspace,
        &provider_id,
        execution_environment,
    )
    .await
    {
        Ok(catalog) => catalog,
        Err(error) => {
            tracing::warn!(
                workspace_id = %workspace.id.0,
                provider_id = provider_id,
                execution_environment = execution_environment.as_str(),
                "failed to load provider model catalog while creating session: {error}"
            );
            cleanup_created_worktree(state, store, workspace, task.id, created_worktree_id).await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let resolved_model = match resolve_model_id(
        Some(req.model_id.as_str()),
        req.reasoning_effort.as_deref(),
        None,
        catalog.as_ref(),
    ) {
        Ok(model) => model,
        Err(_) => {
            cleanup_created_worktree(state, store, workspace, task.id, created_worktree_id).await;
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let model_id = resolved_model.model_id.clone();
    let reasoning_effort = resolved_model.reasoning_effort.clone();
    let preferred_model_id = compose_model_id(&model_id, reasoning_effort.as_deref());

    Ok(PreparedLoadedSessionRequest {
        run_id_header,
        provider_id,
        session_id,
        parent_session_id,
        relationship,
        requested_relationship,
        worktree_id,
        created_worktree_id,
        execution_environment,
        model_id,
        reasoning_effort,
        preferred_model_id,
    })
}

async fn cleanup_created_worktree(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task_id: TaskId,
    created_worktree_id: Option<WorktreeId>,
) {
    if let Some(created_worktree_id) = created_worktree_id {
        cleanup_orphaned_provisioned_worktree(
            state,
            store,
            workspace,
            task_id,
            created_worktree_id,
        )
        .await;
    }
}
