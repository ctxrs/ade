use super::*;
use crate::api::sessions;
use crate::api::shared;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct CreateSessionReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_id")]
    provider_id: String,
    #[serde(deserialize_with = "deserialize_concrete_model_id")]
    model_id: String,
    #[serde(
        default,
        deserialize_with = "sessions::deserialize_optional_reasoning_effort"
    )]
    reasoning_effort: Option<String>,
    #[serde(default)]
    remember_model_preference: bool,
    parent_session_id: Option<String>,
    relationship: Option<String>,
    #[serde(default)]
    initial_prompt: Option<String>,
    #[serde(default)]
    initial_message_id: Option<String>,
    #[serde(default)]
    initial_turn_id: Option<String>,
    #[serde(default)]
    worktree_id: Option<String>,
    #[serde(default)]
    pub(crate) execution_environment: Option<ExecutionEnvironment>,
}

fn deserialize_concrete_model_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("model_id must not be empty"));
    }
    if trimmed.eq_ignore_ascii_case("default") {
        return Err(serde::de::Error::custom(
            "model_id must be a concrete model id",
        ));
    }
    Ok(trimmed.to_string())
}

fn deserialize_provider_id<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(serde::de::Error::custom("provider_id must not be empty"));
    }
    Ok(trimmed.to_string())
}

async fn cleanup_orphaned_provisioned_worktree(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task_id: TaskId,
    worktree_id: WorktreeId,
) {
    let worktree = match store.get_worktree(worktree_id).await {
        Ok(Some(worktree)) => worktree,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree_id.0,
                "failed to load provisioned worktree for rollback cleanup: {err:#}"
            );
            return;
        }
    };
    let sandbox_binding = match store.get_sandbox_binding(worktree_id).await {
        Ok(binding) => binding,
        Err(err) => {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree_id.0,
                "failed to load sandbox binding for provisioned worktree rollback: {err:#}"
            );
            None
        }
    };
    let cleanup_targets = [TaskWorktreeCleanupTarget {
        managed_root: managed_worktree_root(state, workspace, &worktree),
        sandbox_binding,
        worktree: worktree.clone(),
        destroy_worktree_on_cleanup: true,
    }];
    let cleanup_errors =
        cleanup_task_worktrees(state.as_ref(), workspace, task_id, &cleanup_targets).await;
    if !cleanup_errors.is_empty() {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree_id.0,
            cleanup_errors = cleanup_errors.len(),
            "provisioned worktree rollback had cleanup errors"
        );
        return;
    }
    let deleted_worktree_row = match store.delete_worktree(worktree_id).await {
        Ok(deleted) => deleted,
        Err(err) => {
            tracing::warn!(
                task_id = %task_id.0,
                worktree_id = %worktree_id.0,
                "failed to delete provisioned worktree row during rollback: {err:#}"
            );
            false
        }
    };
    if !deleted_worktree_row {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree_id.0,
            "skipping worktree index deletion because provisioned worktree row was not deleted"
        );
        return;
    }
    if let Err(err) = state
        .global_store()
        .delete_workspace_worktree_index(worktree_id)
        .await
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree_id.0,
            "failed to delete provisioned worktree index during rollback: {err:#}"
        );
    }
}

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
    let run_id_header = headers
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let provider_id = req.provider_id.clone();
    if !state
        .providers
        .adapters
        .lock()
        .await
        .contains_key(&provider_id)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let session_id = match req.id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(raw) => Some(SessionId(
            uuid::Uuid::parse_str(raw).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
    };
    let parent_session_id = match req.parent_session_id {
        Some(id) => Some(SessionId(
            uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?,
        )),
        None => None,
    };
    let relationship = req
        .relationship
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let requested_relationship = relationship.clone();
    if parent_session_id.is_some() != relationship.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.initial_prompt.is_some()
        && (req.initial_message_id.is_none() || req.initial_turn_id.is_none())
    {
        state
            .emit_compat_payload_reject_counter("tasks.create_session", "missing_initial_ids", None)
            .await;
        return Err(StatusCode::BAD_REQUEST);
    }

    let workspace_effective =
        execution_effective::effective_execution_settings(&state, workspace.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut existing_worktree = None;
    let worktree_id = if let Some(worktree_id) = req.worktree_id.as_deref() {
        let worktree_id =
            WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?);
        existing_worktree = Some(
            resolve_existing_worktree_execution(&state, &store, &workspace, worktree_id)
                .await
                .map_err(|_| StatusCode::NOT_FOUND)?,
        );
        worktree_id
    } else if let Some(primary) = task.primary_worktree_id {
        existing_worktree = Some(
            resolve_existing_worktree_execution(&state, &store, &workspace, primary)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
        primary
    } else {
        let workspace_root = StdPath::new(&workspace.root_path);
        let vcs = vcs::driver_for_path(workspace_root)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let base_commit_sha = vcs.rev_parse_head(workspace_root).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return StatusCode::BAD_REQUEST;
            }
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        let worktree_id = WorktreeId::new();
        let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
        let (wt_path, sandbox_binding) = provision_worktree_for_execution(
            &state,
            &workspace,
            worktree_id,
            &base_commit_sha,
            &branch_name,
            &workspace_effective,
        )
        .await
        .map_err(|e| {
            tracing::warn!(
                task_id = %task.id.0,
                worktree_id = %worktree_id.0,
                "worktree provisioning failed: {e:#}"
            );
            shared::status_code_for_internal_error(&e)
        })?;

        let worktree = Worktree {
            id: worktree_id,
            workspace_id: task.workspace_id,
            root_path: wt_path.to_string_lossy().to_string(),
            base_commit_sha: base_commit_sha.clone(),
            git_branch: (vcs.kind() == VcsKind::Git).then(|| branch_name.clone()),
            vcs_kind: Some(vcs.kind()),
            base_revision: Some(base_commit_sha.clone()),
            vcs_ref: Some(branch_name.clone()),
            created_at: chrono::Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        };
        persist_provisioned_worktree(&state, &store, &workspace, worktree, sandbox_binding)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        worktree_id
    };
    let created_worktree_id = existing_worktree.is_none().then_some(worktree_id);
    let execution_environment = if let Some(existing) = existing_worktree.as_ref() {
        let persisted = existing.execution_environment();
        if let Some(requested) = req.execution_environment {
            if requested != persisted {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        persisted
    } else {
        let effective_execution_environment =
            execution_environment_from_settings(&workspace_effective);
        match req.execution_environment {
            Some(requested) => {
                if requested != effective_execution_environment {
                    if let Some(created_worktree_id) = created_worktree_id.clone() {
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
                requested
            }
            None => effective_execution_environment,
        }
    };
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
            if let Some(created_worktree_id) = created_worktree_id.clone() {
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
    let resolved_model = match sessions::resolve_model_id(
        Some(req.model_id.as_str()),
        req.reasoning_effort.as_deref(),
        None,
        catalog.as_ref(),
    ) {
        Ok(model) => model,
        Err(_) => {
            if let Some(created_worktree_id) = created_worktree_id.clone() {
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
    let preferred_model_id = sessions::compose_model_id(&model_id, reasoning_effort.as_deref());

    if let Some(session_id) = session_id {
        let existing_ws = state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(existing_ws) = existing_ws {
            if existing_ws != task.workspace_id {
                if let Some(created_worktree_id) = created_worktree_id.clone() {
                    cleanup_orphaned_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        task_id,
                        created_worktree_id,
                    )
                    .await;
                }
                return Err(StatusCode::CONFLICT);
            }
            let existing = store
                .get_session(session_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if let Some(existing) = existing {
                if existing.task_id != task_id
                    || existing.workspace_id != task.workspace_id
                    || existing.worktree_id != worktree_id
                    || existing.execution_environment != execution_environment
                    || existing.provider_id != provider_id
                    || existing.model_id != model_id
                    || existing.reasoning_effort != reasoning_effort
                    || existing.parent_session_id != parent_session_id
                    || existing.relationship != relationship
                {
                    if let Some(created_worktree_id) = created_worktree_id.clone() {
                        cleanup_orphaned_provisioned_worktree(
                            &state,
                            &store,
                            &workspace,
                            task_id,
                            created_worktree_id,
                        )
                        .await;
                    }
                    return Err(StatusCode::CONFLICT);
                }
                state.remember_session_meta(&existing).await;
                if req.remember_model_preference {
                    if let Err(error) =
                        crate::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
                            &state,
                            task.workspace_id,
                            &provider_id,
                            Some(preferred_model_id.clone()),
                        )
                        .await
                    {
                        tracing::warn!(
                            session_id = %existing.id.0,
                            workspace_id = %task.workspace_id.0,
                            provider_id = provider_id,
                            "failed to persist workspace provider model preference: {error:#}"
                        );
                    }
                }
                return Ok(Json(existing));
            }
            if let Some(created_worktree_id) = created_worktree_id.clone() {
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
                if let Some(created_worktree_id) = created_worktree_id.clone() {
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
                if let Some(created_worktree_id) = created_worktree_id.clone() {
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
            || session.task_id != task_id
            || session.workspace_id != task.workspace_id
            || session.worktree_id != worktree_id
            || session.execution_environment != execution_environment
            || session.provider_id != provider_id
            || session.model_id != model_id
            || session.reasoning_effort != reasoning_effort
            || session.parent_session_id != parent_session_id
            || session.relationship != requested_relationship
        {
            return Err(StatusCode::CONFLICT);
        }
    }
    state.remember_session_meta(&session).await;
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

    if let Some(prompt) = req.initial_prompt {
        let (message_id, turn_id) = match (
            req.initial_message_id.as_deref(),
            req.initial_turn_id.as_deref(),
        ) {
            (Some(message_id), Some(turn_id)) => (
                MessageId(uuid::Uuid::parse_str(message_id).map_err(|_| StatusCode::BAD_REQUEST)?),
                TurnId(uuid::Uuid::parse_str(turn_id).map_err(|_| StatusCode::BAD_REQUEST)?),
            ),
            _ => return Err(StatusCode::BAD_REQUEST),
        };

        let delivery = MessageDelivery::Immediate;
        let attachments = Vec::new();
        if let Some(existing) = store
            .get_message(message_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            let matches = existing.session_id == session.id
                && existing.turn_id == Some(turn_id)
                && matches!(existing.role, MessageRole::User)
                && existing.content == prompt
                && existing.attachments.is_empty()
                && matches!(existing.delivery, MessageDelivery::Immediate);
            if matches {
                sessions::ensure_session_turn_for_message(&store, session.id, turn_id, &existing)
                    .await?;
            } else {
                return Err(StatusCode::CONFLICT);
            }
        } else {
            let prompt_for_idempotency = prompt.clone();

            let run_id = RunId::new();
            let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
            let order_seq = {
                let mut order_seq_state = order_seq_state.lock().await;
                order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
            };
            let msg = Message {
                id: message_id,
                session_id: session.id,
                task_id: session.task_id,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                turn_sequence: Some(0),
                order_seq: Some(order_seq),
                role: MessageRole::User,
                content: prompt,
                attachments: attachments.clone(),
                delivery,
                delivered_at: None,
                created_at: chrono::Utc::now(),
            };

            let saved = match store.insert_message(msg).await {
                Ok(saved) => saved,
                Err(err) if is_unique_constraint_violation(&err) => {
                    let Some(existing) = store
                        .get_message(message_id)
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                    else {
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    };
                    let matches = existing.session_id == session.id
                        && existing.turn_id == Some(turn_id)
                        && matches!(existing.role, MessageRole::User)
                        && existing.content == prompt_for_idempotency
                        && existing.attachments.is_empty()
                        && matches!(existing.delivery, MessageDelivery::Immediate);
                    if matches {
                        existing
                    } else {
                        return Err(StatusCode::CONFLICT);
                    }
                }
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
            };

            let event = store
                .append_session_event(
                    session.id,
                    Some(run_id),
                    Some(turn_id),
                    SessionEventType::UserMessage,
                    serde_json::json!({
                        "message_id": saved.id.0,
                        "content": saved.content.clone(),
                        "delivery": saved.delivery.clone(),
                        "attachments": saved.attachments,
                        "order_seq": order_seq,
                    }),
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let start_seq = event.seq;

            let turn = SessionTurn {
                turn_id,
                session_id: session.id,
                run_id: Some(run_id),
                user_message_id: Some(saved.id),
                status: SessionTurnStatus::Starting,
                start_seq: Some(start_seq),
                end_seq: None,
                started_at: saved.created_at,
                updated_at: saved.created_at,
                assistant_partial: None,
                thought_partial: None,
                metrics_json: None,
                tool_total: 0,
                tool_pending: 0,
                tool_running: 0,
                tool_completed: 0,
                tool_failed: 0,
            };

            if let Err(err) = store.insert_session_turn(turn).await {
                if !is_unique_constraint_violation(&err) {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                let existing = store
                    .get_session_turn_by_id(turn_id)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if let Some(existing) = existing {
                    let matches = existing.session_id == session.id
                        && existing.user_message_id == Some(saved.id);
                    if !matches {
                        return Err(StatusCode::CONFLICT);
                    }
                } else {
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }

            state.publish_event(event).await;

            let prompt = saved.content.clone();
            let tx = state.ensure_scheduler(session.clone()).await;
            let queued = crate::scheduler::QueuedMessage {
                message: saved,
                enqueued_at: Instant::now(),
                run_id: run_id_header.clone(),
            };
            let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

            let _ =
                schedule_session_title_generation(state.clone(), session.clone(), prompt, false)
                    .await;
        }
    }

    if req.remember_model_preference {
        if let Err(error) =
            crate::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
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

    let worktree = match state.store_for_session(session.id).await {
        Ok(store) => store.get_worktree(session.worktree_id).await.ok().flatten(),
        Err(_) => None,
    };
    let session_root_kind = session_root_kind_for_worktree(worktree.as_ref()).to_string();
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::session_started(
            session.provider_id.clone(),
            sessions::compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
            Some(session.execution_environment.as_str().to_string()),
            Some(session_root_kind.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": sessions::compose_model_id(&session.model_id, session.reasoning_effort.as_deref()),
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": session.execution_environment.as_str(),
        "session_root_kind": session_root_kind.clone(),
        "parent_session_id": session.parent_session_id.map(|id| id.0.to_string()),
        "relationship": session.relationship.clone(),
    }));
    state.telemetry.ops_events.emit(ops_event);
    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    Ok(Json(session))
}

pub(in crate::api) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let creation_lock = state.task_session_creation_lock(task_id).await;
    let _creation_guard = creation_lock.lock().await;
    create_session_for_task_inner(state, task_id, headers, req).await
}

pub(in crate::api) async fn create_default_session_for_task(
    state: Arc<AppState>,
    task_id: TaskId,
    provider_id: String,
    model_id: String,
    reasoning_effort: Option<String>,
    execution_environment: ExecutionEnvironment,
) -> Result<Session, StatusCode> {
    let Json(session) = create_session_for_task_inner(
        state,
        task_id,
        HeaderMap::new(),
        CreateSessionReq {
            id: None,
            provider_id,
            model_id,
            reasoning_effort,
            remember_model_preference: false,
            parent_session_id: None,
            relationship: None,
            initial_prompt: None,
            initial_message_id: None,
            initial_turn_id: None,
            worktree_id: None,
            execution_environment: Some(execution_environment),
        },
    )
    .await?;
    Ok(session)
}
