use super::*;

pub(in crate::api::tasks) async fn create_session_for_loaded_task_inner(
    state: Arc<AppState>,
    store: Store,
    task: Task,
    workspace: Workspace,
    headers: HeaderMap,
    req: CreateSessionReq,
) -> Result<Json<Session>, StatusCode> {
    let task_id = task.id;
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
        &state,
        &store,
        &task,
        &workspace,
        req.worktree_id.as_deref(),
        req.execution_environment,
    )
    .await?;
    let worktree_id = worktree_resolution.worktree_id;
    let created_worktree_id = worktree_resolution.created_worktree_id;
    let execution_environment = worktree_resolution.execution_environment;
    let catalog = match sessions::load_provider_model_catalog_for_execution_environment(
        &state,
        &workspace,
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
            if let Some(created_worktree_id) = created_worktree_id {
                cleanup_orphaned_provisioned_worktree(
                    &state,
                    &store,
                    &workspace,
                    task_id,
                    created_worktree_id,
                )
                .await;
            }
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
            if let Some(created_worktree_id) = created_worktree_id {
                cleanup_orphaned_provisioned_worktree(
                    &state,
                    &store,
                    &workspace,
                    task_id,
                    created_worktree_id,
                )
                .await;
            }
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    let model_id = resolved_model.model_id.clone();
    let reasoning_effort = resolved_model.reasoning_effort.clone();
    let preferred_model_id = compose_model_id(&model_id, reasoning_effort.as_deref());

    if let Some(existing) = resolve_existing_requested_session(ExistingRequestedSession {
        state: &state,
        store: &store,
        task: &task,
        workspace: &workspace,
        requested_session_id: session_id,
        created_worktree_id,
        identity: SessionCreationIdentity {
            task_id: task.id,
            workspace_id: task.workspace_id,
            worktree_id,
            execution_environment,
            provider_id: &provider_id,
            model_id: &model_id,
            reasoning_effort: reasoning_effort.as_deref(),
            parent_session_id,
            relationship: relationship.as_deref(),
        },
        remember_model_preference: req.remember_model_preference,
        preferred_model_id: &preferred_model_id,
    })
    .await?
    {
        return Ok(Json(existing));
    }

    if let Ok(Some(worktree)) = store.get_worktree(worktree_id).await {
        if let Err(e) =
            vcs_hooks::ensure_task_commit_hook(&state, &workspace, &worktree, task.id).await
        {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %worktree.id.0,
                "failed to configure vcs hooks: {e:#}"
            );
        }
    }

    let requested_session_id = session_id;
    let session = if let Some(session_id) = requested_session_id {
        match store
            .create_session_with_id_and_reasoning_effort(
                session_id,
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                reasoning_effort.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
        {
            Ok(session) => session,
            Err(_) => {
                if let Some(created_worktree_id) = created_worktree_id {
                    cleanup_orphaned_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        task_id,
                        created_worktree_id,
                    )
                    .await;
                }
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    } else {
        match store
            .create_session_with_reasoning_effort(
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                reasoning_effort.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
        {
            Ok(session) => session,
            Err(_) => {
                if let Some(created_worktree_id) = created_worktree_id {
                    cleanup_orphaned_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        task_id,
                        created_worktree_id,
                    )
                    .await;
                }
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    };
    if let Some(session_id) = requested_session_id {
        if session.id != session_id
            || !session_matches_creation_identity(
                &session,
                SessionCreationIdentity {
                    task_id,
                    workspace_id: task.workspace_id,
                    worktree_id,
                    execution_environment,
                    provider_id: &provider_id,
                    model_id: &model_id,
                    reasoning_effort: reasoning_effort.as_deref(),
                    parent_session_id,
                    relationship: requested_relationship.as_deref(),
                },
            )
        {
            return Err(StatusCode::CONFLICT);
        }
    }
    state.sessions.remember_session_meta(&session).await;
    if let Err(e) = retry_global_index_write(|| async {
        state
            .global_store()
            .upsert_workspace_session_index(session.id, task.workspace_id)
            .await
    })
    .await
    {
        tracing::warn!(session_id = %session.id.0, "failed to update session index: {e:?}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if session.parent_session_id.is_none() && session.relationship.is_none() {
        let _ = store
            .set_task_primary_session(task.id, session.id, worktree_id)
            .await;
    }

    seed_initial_prompt(
        &state,
        &store,
        &session,
        InitialPromptSeed {
            prompt: req.initial_prompt,
            message_id: req.initial_message_id,
            turn_id: req.initial_turn_id,
            run_id_header: run_id_header.clone(),
        },
    )
    .await?;

    if req.remember_model_preference {
        if let Err(error) =
            crate::api::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
                &state,
                task.workspace_id,
                &provider_id,
                Some(preferred_model_id),
            )
            .await
        {
            tracing::warn!(
                session_id = %session.id.0,
                workspace_id = %task.workspace_id.0,
                provider_id = provider_id,
                "failed to persist workspace provider model preference: {error:#}"
            );
        }
    }

    emit_session_started_observability(&state, &session, &task).await;

    Ok(Json(session))
}
