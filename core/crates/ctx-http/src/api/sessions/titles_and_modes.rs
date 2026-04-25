use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::super::errors::ApiErrorResp;
use super::{
    compose_model_id, load_provider_model_catalog_for_execution_environment, normalize_effort_id,
    resolve_model_id, store_for_existing_session_api_error,
    store_for_existing_session_api_error_for_write, store_for_existing_session_status_for_write,
};
pub(crate) use crate::daemon::sessions::title_generation::{
    configured_title_generation_settings, maybe_generate_session_title,
    schedule_session_title_generation,
};
#[cfg(test)]
pub(crate) use crate::daemon::sessions::title_generation::{
    generate_title_for_prompt, TitleGenerationSource,
};
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEventType};

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModelReq {
    pub(crate) model_id: String,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
}

pub(crate) async fn set_session_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    fn session_model_error(
        status: StatusCode,
        error: impl Into<String>,
    ) -> (StatusCode, Json<ApiErrorResp>) {
        (
            status,
            Json(ApiErrorResp {
                error: error.into(),
            }),
        )
    }

    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| session_model_error(StatusCode::BAD_REQUEST, "invalid session id"))?,
    );

    let store = store_for_existing_session_api_error_for_write(&state, session_id)
        .await
        .map_err(|(status, resp)| session_model_error(status, resp.0.error))?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "session not found"))?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "workspace not found"))?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "worktree not found"))?;
    let resolved_worktree = crate::api::tasks::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        worktree.id,
    )
    .await
    .map_err(|err| {
        session_model_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            logs::redact_sensitive(&err.to_string()),
        )
    })?;
    let execution_environment = resolved_worktree.execution_environment();
    if session.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %session.id.0,
            stored = session.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "session model update resolved a different execution_environment than persisted metadata"
        );
    }
    let install_target = execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        execution_environment,
    )
    .await
    .map_err(|err| {
        tracing::warn!(
            workspace_id = %worktree.workspace_id.0,
            "set_session_model failed to load execution settings: {err:#}",
        );
        session_model_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to load execution settings",
        )
    })?;

    let adapter = crate::daemon::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await
    .map_err(|err| {
        session_model_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            logs::redact_sensitive(&err.to_string()),
        )
    })?;

    let reasoning_effort = req
        .reasoning_effort
        .as_deref()
        .map(normalize_effort_id)
        .filter(|value| !value.is_empty());
    if let Some(ref effort) = reasoning_effort {
        let allowed = ["none", "minimal", "low", "medium", "high", "xhigh"];
        if !allowed.contains(&effort.as_str()) {
            return Err(session_model_error(
                StatusCode::BAD_REQUEST,
                format!("unsupported reasoning effort '{effort}'"),
            ));
        }
    }
    let catalog = load_provider_model_catalog_for_execution_environment(
        &state,
        &workspace,
        &session.provider_id,
        execution_environment,
    )
    .await
    .map_err(|err| {
        session_model_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            logs::redact_sensitive(&err.to_string()),
        )
    })?;
    let resolved_model = resolve_model_id(
        Some(req.model_id.as_str()),
        reasoning_effort.as_deref(),
        None,
        catalog.as_ref(),
    )
    .map_err(|err| {
        session_model_error(
            StatusCode::BAD_REQUEST,
            logs::redact_sensitive(&err.to_string()),
        )
    })?;
    let next_full_model_id = compose_model_id(
        &resolved_model.model_id,
        resolved_model.reasoning_effort.as_deref(),
    );

    if adapter.has_live_session(&session.id.0.to_string()).await {
        adapter
            .set_session_model(session.id.0.to_string(), next_full_model_id.clone())
            .await
            .map_err(|err| {
                session_model_error(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "failed to switch the live {} session to '{}': {}",
                        session.provider_id,
                        next_full_model_id,
                        logs::redact_sensitive(&err.to_string()),
                    ),
                )
            })?;
    }

    store
        .update_session_model_config(
            session_id,
            resolved_model.model_id.clone(),
            resolved_model.reasoning_effort.clone(),
        )
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?;

    let updated = store
        .get_session(session_id)
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "session not found"))?;
    state.remember_session_meta(&updated).await;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({
                "current_model_id": next_full_model_id,
                "reasoning_effort": resolved_model.reasoning_effort.clone(),
            }),
        )
        .await
        .map_err(|err| {
            session_model_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                logs::redact_sensitive(&err.to_string()),
            )
        })?;
    state.publish_event(event).await;

    if let Err(error) =
        crate::workspace_provider_model_preferences::update_workspace_provider_preferred_model_id(
            &state,
            updated.workspace_id,
            &updated.provider_id,
            Some(next_full_model_id.clone()),
        )
        .await
    {
        tracing::warn!(
            session_id = %updated.id.0,
            workspace_id = %updated.workspace_id.0,
            provider_id = updated.provider_id.as_str(),
            "failed to persist workspace provider model preference after session model update: {error:#}"
        );
    }

    if let Err(e) = state.emit_workspace_task_upsert(updated.task_id).await {
        tracing::warn!(
            task_id = %updated.task_id.0,
            "workspace active snapshot refresh failed after session model update: {e:?}"
        );
    }

    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModeReq {
    pub(crate) mode_id: String,
}

pub(crate) async fn set_session_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModeReq>,
) -> Result<StatusCode, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = store_for_existing_session_status_for_write(&state, session_id).await?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let workspace = state
        .global_store()
        .get_workspace(session.workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let resolved_worktree = crate::api::tasks::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        worktree.id,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let execution_environment = resolved_worktree.execution_environment();
    if session.execution_environment != execution_environment {
        tracing::warn!(
            session_id = %session.id.0,
            stored = session.execution_environment.as_str(),
            resolved = execution_environment.as_str(),
            "session mode update resolved a different execution_environment than persisted metadata"
        );
    }
    let install_target = crate::execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        execution_environment,
    )
    .await
    .map_err(|err| {
        tracing::warn!(
            workspace_id = %worktree.workspace_id.0,
            "set_session_mode failed to load execution settings: {err:#}",
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let adapter = crate::daemon::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    adapter
        .set_session_mode(session.id.0.to_string(), req.mode_id.clone())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({"set_mode": req.mode_id}),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(event).await;

    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub(crate) struct GenerateSessionTitleReq {
    #[serde(default)]
    pub(crate) prompt: Option<String>,
    #[serde(default)]
    pub(crate) force: Option<bool>,
}

pub(crate) async fn generate_session_title(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<GenerateSessionTitleReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let session = store
        .get_session(session_id)
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
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = if let Some(prompt) = req
        .prompt
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
    {
        prompt
    } else {
        store
            .get_first_user_message_content(session_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .filter(|p| !p.trim().is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "prompt required".to_string(),
                }),
            ))?
    };

    let force = req.force.unwrap_or(true);
    let cfg = configured_title_generation_settings(&state).await;
    maybe_generate_session_title(state.clone(), session.clone(), prompt, force, cfg)
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
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "title generation skipped".to_string(),
            }),
        ))?;

    let store = store_for_existing_session_api_error(&state, session_id).await?;
    let updated = store
        .get_session(session_id)
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
                error: "session not found".to_string(),
            }),
        ))?;

    Ok(Json(updated))
}
