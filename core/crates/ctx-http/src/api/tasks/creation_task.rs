use super::*;
use crate::api::shared;

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
    let (wt_path, sandbox_binding) = provision_worktree_for_execution(
        &state,
        &ws,
        worktree_id,
        &base_commit_sha,
        &branch_name,
        &effective,
    )
    .await
    .map_err(|e| shared::map_internal_api_error(&e))?;

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
    let worktree = persist_provisioned_worktree(&state, &store, &ws, worktree, sandbox_binding)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

    if let Err(e) = vcs_hooks::ensure_task_commit_hook(&state, &ws, &worktree, task.id).await {
        tracing::warn!(
            task_id = %task.id.0,
            worktree_id = %worktree_id.0,
            "failed to configure vcs hooks: {e:#}"
        );
    }

    if let Err(e) = store.set_task_primary_worktree(task.id, worktree_id).await {
        tracing::warn!(task_id = %task.id.0, "failed to set primary worktree: {e:?}");
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
