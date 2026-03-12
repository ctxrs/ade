use super::*;
use crate::api::sessions;

pub(in crate::api) async fn create_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTaskReq>,
) -> Result<Json<Task>, (StatusCode, Json<ApiErrorResp>)> {
    let ws_id = WorkspaceId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid workspace id".to_string(),
            }),
        )
    })?);
    let ws = state
        .global_store()
        .get_workspace(ws_id)
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

    let store = state.store_for_workspace(ws_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    let task_id = match req.id.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(raw) => Some(TaskId(uuid::Uuid::parse_str(raw).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid task id".to_string(),
                }),
            )
        })?)),
    };
    if let Some(task_id) = task_id {
        let existing_ws = state
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        if let Some(existing_ws) = existing_ws {
            if existing_ws != ws_id {
                return Err((
                    StatusCode::CONFLICT,
                    Json(ApiErrorResp {
                        error: "task id already exists".to_string(),
                    }),
                ));
            }
            let existing = store.get_task(task_id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            if let Some(existing) = existing {
                if !task_request_matches(&existing, &req.title, &req.description) {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: "task id already exists".to_string(),
                        }),
                    ));
                }
                return Ok(Json(existing));
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "task index exists but task missing".to_string(),
                }),
            ));
        }
    }

    let want_default_session = req.create_default_session;
    let ws_root = StdPath::new(&ws.root_path);
    let vcs = if want_default_session {
        let vcs = vcs::driver_for_path(ws_root).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        vcs.assert_repo(ws_root).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        Some(vcs)
    } else {
        None
    };

    let requested_title = req.title.clone();
    let requested_description = req.description.clone();
    let task = match task_id {
        Some(task_id) => {
            store
                .create_task_with_id(ws_id, task_id, req.title, req.description)
                .await
        }
        None => store.create_task(ws_id, req.title, req.description).await,
    }
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if task_id.is_some() && task.workspace_id != ws_id {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "task id already exists".to_string(),
            }),
        ));
    }
    if task_id.is_some() && !task_request_matches(&task, &requested_title, &requested_description) {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "task id already exists".to_string(),
            }),
        ));
    }
    if let Err(e) = state
        .global_store()
        .upsert_workspace_task_index(task.id, ws_id)
        .await
    {
        tracing::warn!(task_id = %task.id.0, "failed to update task index: {e:?}");
    }

    if !req.create_default_session {
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
    }

    let Some(vcs) = vcs else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: "missing vcs driver for default session".to_string(),
            }),
        ));
    };
    let base_commit_sha = vcs.rev_parse_head(ws_root).await.map_err(|e| {
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

    let worktree_id = WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    let effective = execution_effective::effective_execution_settings(&state, ws.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let wt_path = if matches!(effective.mode, ExecutionMode::Container)
        && matches!(
            effective.container.mount_mode,
            ContainerMountMode::DiskIsolated
        ) {
        if let Err(e) = state
            .execution
            .harness
            .ensure_workspace_container(&ws, &effective, &state.core.daemon_url)
            .await
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            ));
        }
        crate::disk_isolated::ensure_worktree_from_host_copy(
            &state.core.data_root,
            ws_id,
            worktree_id,
            ws_root,
            &base_commit_sha,
            &branch_name,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!(
                        "disk-isolated worktree provisioning failed: {}. retry after checking container runtime health.",
                        logs::redact_sensitive(&e.to_string())
                    ),
                }),
            )
        })?
    } else {
        let wt_path = managed_worktree_path(&state.core.data_root, ws_id, worktree_id);
        if let Some(parent) = wt_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        }
        create_worktree(&ws.root_path, &wt_path, &base_commit_sha, &branch_name)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
        wt_path
    };

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: ws_id,
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
    store.insert_worktree(worktree.clone()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if let Err(e) = state
        .global_store()
        .upsert_workspace_worktree_index(worktree_id, ws_id)
        .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "failed to update worktree index: {e:?}");
    }

    if let Err(e) = vcs_hooks::ensure_task_commit_hook(
        &state.core.data_root,
        ws_id,
        worktree_id,
        StdPath::new(&worktree.root_path),
        worktree.vcs_kind.clone(),
        task.id,
    )
    .await
    {
        tracing::warn!(
            task_id = %task.id.0,
            worktree_id = %worktree_id.0,
            "failed to configure vcs hooks: {e:#}"
        );
    }

    if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(&state),
        ws.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
    }

    if let Err(e) = store.set_task_primary_worktree(task.id, worktree_id).await {
        tracing::warn!(task_id = %task.id.0, "failed to set primary worktree: {e:?}");
    }

    if let Err(e) = attachments::sync_workspace_attachments(Arc::clone(&state), &ws, false).await {
        tracing::warn!(task_id = %task.id.0, "attachment sync failed: {e:?}");
    }

    if let Err(e) =
        attachments::ensure_worktree_attachment_mounts_if_materialized(&state, &ws, &worktree).await
    {
        tracing::warn!(task_id = %task.id.0, "attachment mounts failed: {e:?}");
    }

    let task = match store.get_task_with_activity(task.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })? {
        Some(task) => task,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ))
        }
    };

    if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
        tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
    }
    Ok(Json(task))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::api) struct CreateSessionReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(deserialize_with = "deserialize_provider_id")]
    provider_id: String,
    #[serde(deserialize_with = "deserialize_concrete_model_id")]
    model_id: String,
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
    pub(super) execution_environment: Option<ExecutionEnvironment>,
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

pub(in crate::api) async fn create_session_for_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionReq>,
) -> Result<Json<Session>, StatusCode> {
    let task_id = TaskId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
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
    let model_id = req.model_id.clone();

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
    let effective = execution_effective::effective_execution_settings(&state, workspace.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let effective_execution_environment = execution_environment_from_settings(&effective);
    let execution_environment = match req.execution_environment {
        Some(requested) => {
            if requested != effective_execution_environment {
                return Err(StatusCode::BAD_REQUEST);
            }
            requested
        }
        None => effective_execution_environment,
    };
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

    let worktree_id = if let Some(worktree_id) = req.worktree_id.as_deref() {
        WorktreeId(uuid::Uuid::parse_str(worktree_id).map_err(|_| StatusCode::BAD_REQUEST)?)
    } else if let Some(primary) = task.primary_worktree_id {
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
        let wt_path = if matches!(effective.mode, ExecutionMode::Container)
            && matches!(
                effective.container.mount_mode,
                ContainerMountMode::DiskIsolated
            ) {
            state
                .execution
                .harness
                .ensure_workspace_container(&workspace, &effective, &state.core.daemon_url)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            crate::disk_isolated::ensure_worktree_from_host_copy(
                &state.core.data_root,
                task.workspace_id,
                worktree_id,
                workspace_root,
                &base_commit_sha,
                &branch_name,
            )
            .await
            .map_err(|e| {
                tracing::warn!(
                    task_id = %task.id.0,
                    worktree_id = %worktree_id.0,
                    "disk-isolated worktree provisioning failed: {e:#}"
                );
                StatusCode::INTERNAL_SERVER_ERROR
            })?
        } else {
            let wt_path =
                managed_worktree_path(&state.core.data_root, task.workspace_id, worktree_id);
            if let Some(parent) = wt_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            }
            create_worktree(
                &workspace.root_path,
                &wt_path,
                &base_commit_sha,
                &branch_name,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            wt_path
        };

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
        store
            .insert_worktree(worktree.clone())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Err(e) = retry_global_index_write(|| async {
            state
                .global_store()
                .upsert_workspace_worktree_index(worktree_id, task.workspace_id)
                .await
        })
        .await
        {
            tracing::warn!(
                worktree_id = %worktree_id.0,
                "failed to update worktree index: {e:?}"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
        if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
            Arc::clone(&state),
            workspace.clone(),
            worktree.clone(),
        )
        .await
        {
            tracing::warn!(task_id = %task.id.0, "worktree bootstrap failed: {e:?}");
        }
        if let Err(e) =
            attachments::sync_workspace_attachments(Arc::clone(&state), &workspace, false).await
        {
            tracing::warn!(task_id = %task.id.0, "attachment sync failed: {e:?}");
        }
        if let Err(e) = attachments::ensure_worktree_attachment_mounts_if_materialized(
            &state, &workspace, &worktree,
        )
        .await
        {
            tracing::warn!(task_id = %task.id.0, "attachment mounts failed: {e:?}");
        }
        worktree_id
    };

    if let Some(session_id) = session_id {
        let existing_ws = state
            .global_store()
            .get_workspace_id_for_session(session_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(existing_ws) = existing_ws {
            if existing_ws != task.workspace_id {
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
                    || existing.parent_session_id != parent_session_id
                    || existing.relationship != relationship
                {
                    return Err(StatusCode::CONFLICT);
                }
                state.remember_session_meta(&existing).await;
                return Ok(Json(existing));
            }
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    if let Ok(Some(worktree)) = store.get_worktree(worktree_id).await {
        if let Err(e) = vcs_hooks::ensure_task_commit_hook(
            &state.core.data_root,
            task.workspace_id,
            worktree.id,
            StdPath::new(&worktree.root_path),
            worktree.vcs_kind.clone(),
            task.id,
        )
        .await
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
        store
            .create_session_with_id(
                session_id,
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        store
            .create_session(
                task_id,
                task.workspace_id,
                worktree_id,
                execution_environment,
                provider_id.clone(),
                model_id.clone(),
                "implementer".to_string(),
                parent_session_id,
                relationship.clone(),
                None,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    if let Some(session_id) = requested_session_id {
        if session.id != session_id
            || session.task_id != task_id
            || session.workspace_id != task.workspace_id
            || session.worktree_id != worktree_id
            || session.execution_environment != execution_environment
            || session.provider_id != provider_id
            || session.model_id != model_id
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
                        state
                            .global_store()
                            .upsert_workspace_message_index(existing.id, session.workspace_id)
                            .await
                            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                        existing
                    } else {
                        return Err(StatusCode::CONFLICT);
                    }
                }
                Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
            };
            state
                .global_store()
                .upsert_workspace_message_index(saved.id, session.workspace_id)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
                status: SessionTurnStatus::Running,
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
            session.model_id.clone(),
            Some(session.execution_environment.as_str().to_string()),
            Some(session_root_kind.clone()),
        ))
        .await;
    let mut ops_event = OpsEvent::new("info", "session_started");
    ops_event.session_id = Some(session.id.0.to_string());
    ops_event.worktree_id = Some(session.worktree_id.0.to_string());
    ops_event.provider_id = Some(session.provider_id.clone());
    ops_event.meta = Some(serde_json::json!({
        "model_id": session.model_id.clone(),
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
