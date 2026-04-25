use super::*;
use crate::api::sessions;
use ctx_core::provider_ids::{canonical_provider_id, CODEX_CRP_PROVIDER_ID};
use ctx_provider_install::InstallTarget;

const PREFERRED_DEFAULT_PROVIDER_IDS: &[&str] = &[
    CODEX_CRP_PROVIDER_ID,
    "claude-crp",
    "gemini",
    "qwen",
    "opencode",
    "mistral",
    "kimi",
    "auggie",
];

fn select_default_provider_id(
    statuses: &[ctx_providers::adapters::ProviderStatus],
) -> Option<String> {
    let is_visible = |status: &ctx_providers::adapters::ProviderStatus| {
        !status.detail_flag("ui_hidden").unwrap_or(false)
    };
    let is_installed = |status: &ctx_providers::adapters::ProviderStatus| status.installed;
    let is_ready = |status: &ctx_providers::adapters::ProviderStatus| {
        status.installed
            && status.health == ctx_providers::adapters::ProviderHealth::Ok
            && status.is_usable()
    };
    let mut canonical_statuses = std::collections::BTreeMap::new();
    for status in statuses {
        canonical_statuses
            .entry(canonical_provider_id(&status.provider_id).to_string())
            .or_insert(status);
    }

    for preferred in PREFERRED_DEFAULT_PROVIDER_IDS {
        if canonical_statuses
            .get(*preferred)
            .is_some_and(|status| is_visible(status) && is_ready(status))
        {
            return Some((*preferred).to_string());
        }
    }

    canonical_statuses
        .iter()
        .filter(|(_, status)| is_visible(status) && is_ready(status))
        .map(|(provider_id, _)| provider_id.clone())
        .next()
        .or_else(|| {
            canonical_statuses
                .iter()
                .filter(|(_, status)| is_ready(status))
                .map(|(provider_id, _)| provider_id.clone())
                .next()
        })
        .or_else(|| {
            canonical_statuses
                .iter()
                .filter(|(_, status)| is_visible(status) && is_installed(status))
                .map(|(provider_id, _)| provider_id.clone())
                .next()
        })
        .or_else(|| {
            canonical_statuses
                .iter()
                .filter(|(_, status)| is_installed(status))
                .map(|(provider_id, _)| provider_id.clone())
                .next()
        })
}

async fn validate_workspace_root_is_repo(
    workspace: &Workspace,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let workspace_root = StdPath::new(&workspace.root_path);
    let vcs = vcs::driver_for_path(workspace_root).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    vcs.assert_repo(workspace_root).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: e.to_string(),
            }),
        )
    })?;
    Ok(())
}

async fn preflight_default_session_creation(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
) -> Result<(ExecutionEnvironment, String, String, Option<String>), (StatusCode, Json<ApiErrorResp>)>
{
    validate_workspace_root_is_repo(workspace).await?;
    let effective = execution_effective::effective_execution_settings(state, workspace.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let execution_environment = execution_environment_from_settings(&effective);
    let (provider_id, model_id, reasoning_effort) =
        resolve_default_session_target(state, store, workspace, execution_environment).await?;
    Ok((
        execution_environment,
        provider_id,
        model_id,
        reasoning_effort,
    ))
}

async fn resolve_default_session_target(
    state: &Arc<AppState>,
    store: &Store,
    workspace: &Workspace,
    execution_environment: ExecutionEnvironment,
) -> Result<(String, String, Option<String>), (StatusCode, Json<ApiErrorResp>)> {
    let install_target = crate::api::providers::install_target_for_workspace(state, workspace.id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&error.to_string()),
                }),
            )
        })?;
    let mut statuses =
        crate::api::providers::providers_statuses_response(state, install_target, true).await;
    let provider_id = match select_default_provider_id(&statuses) {
        Some(provider_id) => provider_id,
        None if install_target == InstallTarget::Host => {
            crate::installer::refresh_provider_statuses(state.as_ref())
                .await
                .map_err(|error| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&error.to_string()),
                        }),
                    )
                })?;
            statuses =
                crate::api::providers::providers_statuses_response(state, install_target, true)
                    .await;
            select_default_provider_id(&statuses).ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "no provider available for default session".to_string(),
                }),
            ))?
        }
        None => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "no provider available for default session".to_string(),
                }),
            ));
        }
    };
    let provider_status = statuses
        .iter()
        .find(|status| canonical_provider_id(&status.provider_id) == provider_id);
    let preferred_model_id =
        ctx_workspace_config::load_preferred_new_session_model_id(store, &provider_id)
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&error.to_string()),
                    }),
                )
            })?;
    let catalog = sessions::load_provider_model_catalog_for_execution_environment(
        state,
        workspace,
        &provider_id,
        execution_environment,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&error),
            }),
        )
    })?;
    let fallback_model = catalog
        .as_ref()
        .and_then(sessions::ModelCatalog::default_model_id)
        .map(str::to_string)
        .or_else(|| {
            provider_status.and_then(|status| {
                crate::api::provider_launch::subscription_models_payload_from_status(status)
                    .and_then(|value| {
                        value
                            .get("current_model_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
            })
        });
    let resolved_model = sessions::resolve_model_id(
        preferred_model_id.as_deref(),
        None,
        fallback_model.as_deref(),
        catalog.as_ref(),
    )
    .map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "failed to resolve default model for provider '{provider_id}': {error}"
                ),
            }),
        )
    })?;
    Ok((
        provider_id,
        resolved_model.model_id,
        resolved_model.reasoning_effort,
    ))
}

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
    if req.create_default_session && existing_task.is_none() {
        preflight_default_session_creation(&state, &store, &ws).await?;
    }
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

    if !req.create_default_session {
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
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
    if task.primary_session_id.is_some() {
        if let Err(e) = state.emit_workspace_task_upsert(task.id).await {
            tracing::warn!(task_id = %task.id.0, "workspace active snapshot refresh failed: {e:?}");
        }
        return Ok(Json(task));
    }
    let (execution_environment, provider_id, model_id, reasoning_effort) =
        match preflight_default_session_creation(&state, &store, &ws).await {
            Ok(plan) => plan,
            Err(err) => {
                if created_task_in_this_request {
                    rollback_new_task_after_default_session_failure(&state, &store, &ws, task.id)
                        .await;
                }
                return Err(err);
            }
        };
    if let Err(status) = create_default_session_for_task(
        Arc::clone(&state),
        task.id,
        provider_id,
        model_id,
        reasoning_effort,
        execution_environment,
    )
    .await
    {
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

#[cfg(test)]
mod tests {
    use super::select_default_provider_id;
    use ctx_providers::adapters::{
        ProviderHealth, ProviderRecommendedAction, ProviderStatus, ProviderUsability,
        ProviderUsabilityStatus,
    };
    use std::collections::HashMap;

    fn status(provider_id: &str) -> ProviderStatus {
        ProviderStatus {
            provider_id: provider_id.to_string(),
            installed: true,
            detected_path: None,
            version: None,
            capabilities: None,
            health: ProviderHealth::Ok,
            diagnostics: Vec::new(),
            details: HashMap::new(),
            usability: ProviderUsability {
                usable: true,
                status: ProviderUsabilityStatus::Ready,
                reason_code: None,
                reason: None,
                blocking_provider_ids: Vec::new(),
                recommended_action: ProviderRecommendedAction::None,
            },
        }
    }

    #[test]
    fn selects_canonical_codex_crp_for_default_session_creation() {
        let statuses = vec![status("codex")];

        assert_eq!(
            select_default_provider_id(&statuses),
            Some("codex-crp".to_string())
        );
    }

    #[test]
    fn falls_back_to_stable_canonical_order_for_nonpreferred_providers() {
        let statuses = vec![status("zeta"), status("alpha")];

        assert_eq!(
            select_default_provider_id(&statuses),
            Some("alpha".to_string())
        );
    }
}
