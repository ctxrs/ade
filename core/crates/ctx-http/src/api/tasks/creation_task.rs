use super::*;

#[path = "creation_task/default_session_plan.rs"]
mod default_session_plan;
use default_session_plan::preflight_default_session_creation;

async fn rollback_new_task_after_default_session_failure(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    task_id: TaskId,
) {
    let task = match store.get_task(task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(
                task_id = %task_id.0,
                "failed to load task while rolling back brand-new task: {err:#}"
            );
            return;
        }
    };
    match delete_loaded_task_with_cleanup(state, store, workspace, &task).await {
        Ok(()) | Err(StatusCode::NOT_FOUND) => {}
        Err(status) => {
            tracing::warn!(
                task_id = %task_id.0,
                ?status,
                "failed to rollback brand-new task after default-session creation failure"
            );
        }
    }
}

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
    let requested_title = req.title.clone();
    let requested_description = req.description.clone();
    let requested_default_session = req.default_session.clone();
    let existing_task = match task_id {
        Some(task_id) => {
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
                let Some(existing) = existing else {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "task index exists but task missing".to_string(),
                        }),
                    ));
                };
                if !task_request_matches(&existing, &requested_title, &requested_description) {
                    return Err((
                        StatusCode::CONFLICT,
                        Json(ApiErrorResp {
                            error: "task id already exists".to_string(),
                        }),
                    ));
                }
                Some(existing)
            } else {
                None
            }
        }
        None => None,
    };
    let default_session_plan = if existing_task.is_none() && requested_default_session.is_none() {
        Some(preflight_default_session_creation(&state, &store, &ws).await?)
    } else {
        None
    };
    let (task, mut created_task_in_this_request) = match existing_task {
        Some(existing) => (existing, false),
        None => match task_id {
            Some(task_id) => {
                let result = store
                    .create_task_with_id_result(ws_id, task_id, req.title, req.description)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: logs::redact_sensitive(&e.to_string()),
                            }),
                        )
                    })?;
                (result.task, result.created)
            }
            None => (
                store
                    .create_task(ws_id, req.title, req.description)
                    .await
                    .map_err(|e| {
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiErrorResp {
                                error: logs::redact_sensitive(&e.to_string()),
                            }),
                        )
                    })?,
                true,
            ),
        },
    };
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

    let default_session_lock = state.task_session_creation_lock(task.id).await;
    let _default_session_guard = default_session_lock.lock().await;
    let task = store.get_task_with_activity(task.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let task = if let Some(task) = task {
        task
    } else if !created_task_in_this_request {
        let Some(task_id) = task_id else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "task not found".to_string(),
                }),
            ));
        };
        let retry = store
            .create_task_with_id_result(
                ws_id,
                task_id,
                requested_title.clone(),
                requested_description.clone(),
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
        if !retry.created
            && (retry.task.workspace_id != ws_id
                || !task_request_matches(&retry.task, &requested_title, &requested_description))
        {
            return Err((
                StatusCode::CONFLICT,
                Json(ApiErrorResp {
                    error: "task id already exists".to_string(),
                }),
            ));
        }
        created_task_in_this_request = retry.created;
        retry.task
    } else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "task not found".to_string(),
            }),
        ));
    };
    if let Some(primary_session_id) = task.primary_session_id {
        if let Some(default_session_req) = requested_default_session {
            if let Err(status) = super::session_creation::replay_requested_default_session_for_task(
                Arc::clone(&state),
                store.clone(),
                task.clone(),
                ws.clone(),
                default_session_req,
                primary_session_id,
            )
            .await
            {
                return Err((
                    status,
                    Json(ApiErrorResp {
                        error: "task id already exists with a different default session"
                            .to_string(),
                    }),
                ));
            }
        }
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
    }
    let default_session_result = if let Some(default_session_req) = requested_default_session {
        super::session_creation::create_requested_default_session_for_task(
            Arc::clone(&state),
            store.clone(),
            task.clone(),
            ws.clone(),
            default_session_req,
        )
        .await
    } else {
        let (execution_environment, provider_id, model_id, reasoning_effort) =
            match default_session_plan {
                Some(plan) => plan,
                None => match preflight_default_session_creation(&state, &store, &ws).await {
                    Ok(plan) => plan,
                    Err(err) => {
                        if created_task_in_this_request {
                            rollback_new_task_after_default_session_failure(
                                &state, &store, &ws, task.id,
                            )
                            .await;
                        }
                        return Err(err);
                    }
                },
            };
        create_default_session_for_task(
            Arc::clone(&state),
            store.clone(),
            task.clone(),
            ws.clone(),
            DefaultSessionSeed {
                provider_id,
                model_id,
                reasoning_effort,
                execution_environment,
            },
        )
        .await
    };
    if let Err(status) = default_session_result {
        if created_task_in_this_request {
            rollback_new_task_after_default_session_failure(&state, &store, &ws, task.id).await;
        }
        return Err((
            status,
            Json(ApiErrorResp {
                error: "failed to create default session".to_string(),
            }),
        ));
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
