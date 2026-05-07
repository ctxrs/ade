use super::*;

pub(crate) async fn init_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: AgentInitReq,
) -> ApiResult<Vec<SpawnedChild>> {
    if req.agents.is_empty() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "agents is required",
        ));
    }

    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(internal_api_error)?;
    let max_subagents =
        resolve_max_subagents_per_call(settings.subagents.as_ref().and_then(|s| s.max_per_call));
    if req.agents.len() > max_subagents {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!("max {max_subagents} subagents per call"),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            "response_mode is not supported; use wait_agent to await",
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let creation_lock = state.task_session_creation_lock(parent.task_id).await;
    let _creation_guard = creation_lock.lock().await;
    if parent.parent_session_id.is_some() {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "subagents cannot spawn child agents; max depth is {}",
                DEFAULT_MAX_SUBAGENT_DEPTH
            ),
        ));
    }
    let existing_active = store
        .count_active_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;
    if existing_active + req.agents.len() > DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT {
        return Err(api_error(
            SubagentErrorKind::BadRequest,
            format!(
                "max {} active child agents per parent",
                DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT
            ),
        ));
    }
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "workspace not found"))?;

    let labels = validate_requested_labels(&store, parent.task_id, &req.agents).await?;
    let request_agents = req
        .agents
        .iter()
        .map(|agent| SubagentRequestAgent {
            prompt: &agent.prompt,
            label: agent.label.as_deref(),
            harness: agent.harness.as_deref(),
            model: agent.model.as_deref(),
            reasoning_effort: agent.reasoning_effort.as_deref(),
        })
        .collect::<Vec<_>>();
    let provider_ids = collect_provider_ids(&request_agents, &parent.provider_id)
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

    let parent_worktree_execution = crate::api::tasks::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        parent.worktree_id,
    )
    .await
    .map_err(internal_api_error)?;
    let parent_worktree = parent_worktree_execution.worktree.clone();
    let resolved_parent_execution_environment = parent_worktree_execution.execution_environment();
    if parent.execution_environment != resolved_parent_execution_environment {
        tracing::warn!(
            session_id = %parent.id.0,
            stored = parent.execution_environment.as_str(),
            resolved = resolved_parent_execution_environment.as_str(),
            "parent session execution_environment drifted from resolved worktree identity"
        );
    }

    let model_catalogs = load_requested_model_catalogs(
        &state,
        &workspace,
        &provider_ids,
        resolved_parent_execution_environment,
    )
    .await?;
    let worktree_plan =
        plan_subagent_worktree_creation(&state, &parent_worktree, worktree_selection).await?;

    let request_json = Some(build_subagent_request_json(&request_agents));
    let mut requested_tool_call_id = req
        .tool_call_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let invocation_id = requested_tool_call_id
        .clone()
        .unwrap_or_else(|| format!("subagent-{}", uuid::Uuid::new_v4()));
    let tool_call_id = requested_tool_call_id
        .take()
        .unwrap_or_else(|| invocation_id.clone());

    let mut parent_turn_id = None;
    if !tool_call_id.trim().is_empty() {
        if let Ok(Some(tool)) = store.get_session_turn_tool(parent.id, &tool_call_id).await {
            parent_turn_id = Some(tool.turn_id);
        }
    }
    if parent_turn_id.is_none() {
        if let Ok(turns) = store
            .list_session_turns_page_by_seq(parent.id, None, Some(5))
            .await
        {
            for turn in turns.iter().rev() {
                if matches!(
                    turn.status,
                    SessionTurnStatus::Starting
                        | SessionTurnStatus::Running
                        | SessionTurnStatus::Queued
                ) {
                    parent_turn_id = Some(turn.turn_id);
                    break;
                }
            }
        }
    }

    let now = chrono::Utc::now();
    let invocation = SubagentInvocation {
        id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        parent_session_id: parent.id,
        parent_turn_id,
        requested_count: req.agents.len() as i64,
        request_json,
        status: "requested".to_string(),
        created_at: now,
        updated_at: now,
        children: Vec::new(),
    };
    store
        .upsert_subagent_invocation(invocation)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_created",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "requested",
            "requested_count": req.agents.len(),
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let running_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(&invocation_id, "running", running_at)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "running",
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let child_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let parent_effective = parent_worktree_execution.effective.clone();

    let mut futures = Vec::with_capacity(req.agents.len());
    for (idx, agent) in req.agents.into_iter().enumerate() {
        let state = state.clone();
        let parent = parent.clone();
        let workspace = workspace.clone();
        let model_catalogs = model_catalogs.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let child_ids = child_ids.clone();
        let parent_turn_id = parent_turn_id;
        let label = labels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("Subagent {}", idx + 1));
        let worktree_plan = worktree_plan.clone();
        let parent_effective = parent_effective.clone();
        futures.push(async move {
            let store = state
                .store_for_session(parent.id)
                .await
                .map_err(internal_api_error)?;
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err(api_error(
                    SubagentErrorKind::BadRequest,
                    format!("agent {} prompt is required", idx + 1),
                ));
            }
            let harness_defaulted = agent.harness.is_none();
            let provider_id = agent
                .harness
                .as_deref()
                .unwrap_or(&parent.provider_id)
                .trim()
                .to_string();
            if harness_defaulted {
                state
                    .emit_product_fallback_applied_counter(
                        "sessions.subagent_init",
                        "harness_default_parent",
                        None,
                    )
                    .await;
            }
            let catalog = model_catalogs
                .get(&provider_id)
                .and_then(|value| value.as_ref());
            let fallback_model = if agent.model.is_none() {
                if provider_id == parent.provider_id {
                    Some(parent.model_id.as_str())
                } else {
                    default_catalog_model_id(catalog)
                }
            } else {
                None
            };
            if agent.model.is_none() && fallback_model.is_none() {
                state
                    .emit_compat_payload_reject_counter(
                        "sessions.subagent_init",
                        "missing_model_without_default",
                        Some(("provider_id", &provider_id)),
                    )
                    .await;
                return Err(api_error(
                    SubagentErrorKind::BadRequest,
                    format!("model is required for harness '{provider_id}'"),
                ));
            }
            if agent.model.is_none() {
                let fallback = if provider_id == parent.provider_id {
                    "model_default_parent"
                } else {
                    "model_default_catalog"
                };
                state
                    .emit_product_fallback_applied_counter("sessions.subagent_init", fallback, None)
                    .await;
            }
            let resolved = resolve_model_id(
                agent.model.as_deref(),
                agent.reasoning_effort.as_deref(),
                fallback_model,
                catalog,
            )
            .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;

            let prompt_length = prompt.chars().count() as i64;
            let reasoning_effort = resolved.reasoning_effort.clone();

            let (worktree_id, worktree_path) = match worktree_selection {
                SubagentWorktreeSelection::Inherit => (parent.worktree_id, None),
                SubagentWorktreeSelection::New => {
                    let (vcs_kind, base_commit_sha) = worktree_plan.clone().ok_or_else(|| {
                        api_error(SubagentErrorKind::Internal, "worktree plan missing")
                    })?;
                    let worktree = create_subagent_worktree(
                        &state,
                        &store,
                        &workspace,
                        parent.task_id,
                        &base_commit_sha,
                        vcs_kind,
                        &parent_effective,
                    )
                    .await?;
                    (worktree.id, Some(worktree.root_path))
                }
            };

            let session = store
                .create_session_with_reasoning_effort(
                    parent.task_id,
                    parent.workspace_id,
                    worktree_id,
                    resolved_parent_execution_environment,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    reasoning_effort.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(internal_api_error)?;
            if let Err(error) = state
                .global_store()
                .upsert_workspace_session_index(session.id, parent.workspace_id)
                .await
            {
                tracing::warn!(
                    session_id = %session.id.0,
                    "failed to update subagent session index: {error:?}"
                );
            }
            if store
                .update_session_title(session.id, label.clone())
                .await
                .is_err()
            {
                tracing::warn!(session_id = %session.id.0, "failed to set subagent label");
            }

            let child_created_at = chrono::Utc::now();
            let persisted = persist_subagent_prompt(&state, &session, prompt).await?;
            let child_session_id = session.id;
            let child = SubagentInvocationChild {
                invocation_id: invocation_id.clone(),
                child_session_id,
                run_id: Some(persisted.run_id),
                position: idx as i64,
                status: "running".to_string(),
                label: Some(label),
                harness: Some(provider_id),
                model: Some(resolved.full_model_id),
                reasoning_effort,
                prompt_length,
                created_at: child_created_at,
                updated_at: child_created_at,
            };
            store
                .upsert_subagent_invocation_child(child.clone())
                .await
                .map_err(internal_api_error)?;

            let child_ids_snapshot = {
                let mut ids = child_ids.lock().await;
                let child_id_string = child_session_id.0.to_string();
                ids.push(child_id_string);
                ids.clone()
            };
            emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "running",
                    "child_session_ids": child_ids_snapshot,
                }),
            )
            .await?;
            dispatch_subagent_prompt(&state, &session, &persisted.saved_message).await;

            Ok(SpawnedChild {
                child,
                worktree_path,
                last_event_seq: persisted.last_event_seq,
            })
        });
    }

    let spawned_children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(error) => {
            let updated_at = chrono::Utc::now();
            if let Ok(store) = state.store_for_session(parent.id).await {
                if let Err(update_error) = store
                    .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                    .await
                {
                    tracing::warn!(
                        error = ?update_error,
                        "failed to update subagent invocation status"
                    );
                }
            }
            let child_session_ids = {
                let ids = child_ids.lock().await;
                ids.clone()
            };
            let _ = emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "failed",
                    "child_session_ids": child_session_ids,
                }),
            )
            .await;
            return Err(error);
        }
    };

    for spawned in spawned_children.iter().cloned() {
        let state_weak = Arc::downgrade(&state);
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let parent_id = parent.id;
        let parent_worktree_id = parent.worktree_id;
        tokio::spawn(async move {
            if let Err(error) =
                run_subagent_child(&state_weak, spawned.child, parent_worktree_id).await
            {
                tracing::warn!(error = %error, "subagent execution failed");
            }
            if let Some(state) = state_weak.upgrade() {
                if let Err(error) = finalize_subagent_invocation(
                    &state,
                    &invocation_id,
                    &tool_call_id,
                    parent_id,
                    parent_turn_id,
                )
                .await
                {
                    tracing::warn!(error = %error, "failed to finalize subagent invocation");
                }
            }
        });
    }

    Ok(spawned_children)
}
