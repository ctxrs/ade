use super::*;

fn default_catalog_model_id(catalog: Option<&ModelCatalog>) -> Option<&str> {
    catalog.and_then(ModelCatalog::default_model_id)
}

pub(crate) async fn mcp_agent_init(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentInitReq>,
) -> Result<Json<AgentInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if req.agents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "agents is required".to_string(),
            }),
        ));
    }
    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let max_subagents = resolve_max_subagents_per_call(&settings);
    if req.agents.len() > max_subagents {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("max {max_subagents} subagents per call"),
            }),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "response_mode is not supported; use subagent_wait to await".to_string(),
            }),
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let mut labels = Vec::with_capacity(req.agents.len());
    let mut seen_labels = HashSet::new();
    for (idx, agent) in req.agents.iter().enumerate() {
        let label = agent
            .label
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("agent {} label is required", idx + 1),
                }),
            ))?;
        if !seen_labels.insert(label.to_string()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate subagent label '{label}'"),
                }),
            ));
        }
        labels.push(label.to_string());
    }
    for label in &labels {
        if store
            .subagent_label_exists(parent.task_id, label)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent label '{label}' already exists for this task"),
                }),
            ));
        }
    }

    let mut provider_ids = HashSet::new();
    for agent in &req.agents {
        let provider_id = agent
            .harness
            .as_deref()
            .unwrap_or(&parent.provider_id)
            .trim();
        if provider_id.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "harness is required".to_string(),
                }),
            ));
        }
        provider_ids.insert(provider_id.to_string());
    }

    let available_providers: Vec<String> = {
        let statuses = state.providers.statuses.lock().await;
        let mut ids = statuses.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    };
    let mut provider_statuses = HashMap::new();
    {
        let statuses = state.providers.statuses.lock().await;
        for provider_id in provider_ids.iter() {
            if let Some(status) = statuses.get(provider_id) {
                provider_statuses.insert(provider_id.clone(), status.clone());
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!(
                            "unknown harness '{provider_id}'; available harnesses: {}",
                            available_providers.join(", ")
                        ),
                    }),
                ));
            }
        }
    }

    for (provider_id, status) in &provider_statuses {
        if !crate::provider_usability::provider_status_is_usable(status) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "harness '{provider_id}' is not ready: {}",
                        crate::provider_usability::provider_status_unusable_reason(status)
                            .unwrap_or_else(|| "provider not ready for use".to_string())
                    ),
                }),
            ));
        }
    }

    let mut model_catalogs: HashMap<String, Option<ModelCatalog>> = HashMap::new();
    for provider_id in provider_ids.iter() {
        let catalog = load_provider_model_catalog(&state, &workspace, provider_id).await;
        match catalog {
            Ok(cat) => {
                model_catalogs.insert(provider_id.clone(), cat);
            }
            Err(err) => {
                return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: err })));
            }
        }
    }

    let parent_worktree = store
        .get_worktree(parent.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent worktree not found".to_string(),
            }),
        ))?;

    let worktree_plan = if worktree_selection == SubagentWorktreeSelection::New {
        let parent_root = StdPath::new(&parent_worktree.root_path);
        let vcs = vcs::driver_for_path(parent_root).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        let base_commit_sha = vcs.rev_parse_head(parent_root).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "git repo has no commits; create an initial commit before creating a worktree".to_string(),
                    }),
                );
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

        let (dirty_files, dirty_additions, dirty_deletions) =
            ctx_fs::worktrees::diff_worktree_summary(parent_root, &base_commit_sha)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        if dirty_files > 0 || dirty_additions > 0 || dirty_deletions > 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs.".to_string(),
                }),
            ));
        }
        Some((vcs.kind(), base_commit_sha))
    } else {
        None
    };

    let request_json = Some(build_subagent_request_json(&req.agents));

    let mut requested_tool_call_id = req
        .tool_call_id
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
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
                    SessionTurnStatus::Running | SessionTurnStatus::Queued
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state
        .global_store()
        .upsert_workspace_subagent_invocation_index(&invocation_id, parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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
        futures.push(async move {
            let store = state.store_for_session(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("agent {} prompt is required", idx + 1),
                    }),
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
            let catalog = model_catalogs.get(&provider_id).and_then(|v| v.as_ref());
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
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("model is required for harness '{provider_id}'"),
                    }),
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
            .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

            let prompt_length = prompt.chars().count() as i64;
            let reasoning_effort = resolved.reasoning_effort.clone();

            let worktree_id = match worktree_selection {
                SubagentWorktreeSelection::Inherit => parent.worktree_id,
                SubagentWorktreeSelection::New => {
                    let (vcs_kind, base_commit_sha) = worktree_plan.clone().ok_or((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "worktree plan missing".to_string(),
                        }),
                    ))?;
                    let worktree = create_subagent_worktree(
                        &state,
                        &store,
                        &workspace,
                        parent.task_id,
                        &base_commit_sha,
                        vcs_kind,
                    )
                    .await?;
                    worktree.id
                }
            };

            let session = store
                .create_session_with_reasoning_effort(
                    parent.task_id,
                    parent.workspace_id,
                    worktree_id,
                    parent.execution_environment,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    reasoning_effort.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
            if let Err(e) = state
                .global_store()
                .upsert_workspace_session_index(session.id, parent.workspace_id)
                .await
            {
                tracing::warn!(
                    session_id = %session.id.0,
                    "failed to update subagent session index: {e:?}"
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
            let (run_id, _message) = enqueue_subagent_prompt(&state, &session, prompt).await?;
            let child_session_id = session.id;
            let child = SubagentInvocationChild {
                invocation_id: invocation_id.clone(),
                child_session_id,
                run_id: Some(run_id),
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
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;

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

            Ok(child)
        });
    }

    let children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(err) => {
            let updated_at = chrono::Utc::now();
            if let Ok(store) = state.store_for_session(parent.id).await {
                if let Err(e) = store
                    .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                    .await
                {
                    tracing::warn!(error = ?e, "failed to update subagent invocation status");
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
            return Err(err);
        }
    };

    for child in children.iter().cloned() {
        let state = state.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let parent_id = parent.id;
        let parent_worktree_id = parent.worktree_id;
        tokio::spawn(async move {
            if let Err(error) = run_subagent_child(&state, child, parent_worktree_id).await {
                tracing::warn!(error = %error, "subagent execution failed");
            }
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
        });
    }

    let results = futures::future::try_join_all(children.iter().map(|child| {
        let context_window = match (child.harness.as_deref(), child.model.as_deref()) {
            (Some(provider_id), Some(model_id)) => {
                estimate_context_window_for_prompt_len(provider_id, model_id, child.prompt_length)
            }
            _ => None,
        };
        build_subagent_result(
            &state,
            parent.worktree_id,
            child,
            "running".to_string(),
            None,
            context_window,
        )
    }))
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        )
    })?;

    Ok(Json(AgentInitResp {
        status: "running".to_string(),
        results,
    }))
}
