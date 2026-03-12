use std::path::Path as StdPath;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::settings as user_settings;
use crate::title_generation;
use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEventType};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TitleGenerationSource {
    Llm,
    Fallback,
}

impl TitleGenerationSource {
    fn as_str(self) -> &'static str {
        match self {
            TitleGenerationSource::Llm => "llm",
            TitleGenerationSource::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TitleGenerationOutcome {
    pub(crate) title: String,
    pub(crate) source: TitleGenerationSource,
}

pub(crate) async fn configured_title_generation_settings(
    state: &AppState,
) -> Option<user_settings::TitleGenerationSettings> {
    let settings = match user_settings::load_settings(state.global_store()).await {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(
                "failed to load title-generation settings: {}",
                logs::redact_sensitive(&err.to_string())
            );
            return None;
        }
    };
    settings
        .title_generation
        .as_ref()
        .filter(|cfg| title_generation::is_configured(cfg))
        .cloned()
}

pub(crate) async fn generate_title_for_prompt(
    cfg: Option<&user_settings::TitleGenerationSettings>,
    prompt: &str,
    data_root: &StdPath,
) -> anyhow::Result<TitleGenerationOutcome> {
    let fallback = title_generation::fallback_title_from_prompt(prompt);
    if fallback.trim().is_empty() {
        return Err(anyhow::anyhow!("prompt is empty"));
    }

    if let Some(cfg) = cfg.filter(|c| title_generation::is_configured(c)) {
        match title_generation::generate_title(cfg, prompt, data_root).await {
            Ok(title) => {
                return Ok(TitleGenerationOutcome {
                    title,
                    source: TitleGenerationSource::Llm,
                });
            }
            Err(err) => {
                tracing::warn!(
                    "title generation failed: {}",
                    logs::redact_sensitive(&err.to_string())
                );
                // TODO: surface a snackbar/toast when title generation fails.
            }
        }
    }

    Ok(TitleGenerationOutcome {
        title: fallback,
        source: TitleGenerationSource::Fallback,
    })
}

pub(crate) async fn apply_session_title_update(
    state: &Arc<AppState>,
    session: &Session,
    outcome: TitleGenerationOutcome,
) -> anyhow::Result<()> {
    let store = state.store_for_session(session.id).await?;
    let updated = store
        .update_session_title(session.id, outcome.title.clone())
        .await
        .context("updating session title")?;
    if !updated {
        return Ok(());
    }

    if let Ok(Some(updated_session)) = store.get_session(session.id).await {
        state.remember_session_meta(&updated_session).await;
    }

    if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
        tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
    }

    let mut task_updated = false;
    if let Ok(Some(task)) = store.get_task(session.task_id).await {
        let title = task.title.trim();
        if (title.is_empty() || title == title_generation::DEFAULT_SESSION_TITLE)
            && store
                .update_task_title(session.task_id, outcome.title.clone())
                .await
                .unwrap_or(false)
        {
            task_updated = true;
        }
    }

    if task_updated {
        if let Err(e) = state.emit_workspace_task_upsert(session.task_id).await {
            tracing::warn!(task_id = %session.task_id.0, "workspace active snapshot refresh failed: {e:?}");
        }
    }

    let notice = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::Notice,
            serde_json::json!({
                "kind": "title_generated",
                "title": outcome.title,
                "source": outcome.source.as_str(),
            }),
        )
        .await;
    if let Ok(event) = notice {
        state.publish_event(event).await;
    }

    Ok(())
}

pub(crate) async fn maybe_generate_session_title(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
    cfg: Option<user_settings::TitleGenerationSettings>,
) -> anyhow::Result<Option<TitleGenerationOutcome>> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() {
        return Ok(None);
    }

    let current = session.title.trim();
    if !force && !current.is_empty() && current != title_generation::DEFAULT_SESSION_TITLE {
        return Ok(None);
    }

    let outcome = generate_title_for_prompt(cfg.as_ref(), &prompt, &state.core.data_root).await?;
    apply_session_title_update(&state, &session, outcome.clone()).await?;
    Ok(Some(outcome))
}

pub(crate) async fn schedule_session_title_generation(
    state: Arc<AppState>,
    session: Session,
    prompt: String,
    force: bool,
) -> bool {
    let cfg = configured_title_generation_settings(&state).await;
    if cfg.is_some() {
        tokio::spawn(async move {
            let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        });
        true
    } else {
        let _ = maybe_generate_session_title(state, session, prompt, force, cfg).await;
        false
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModelReq {
    pub(crate) model_id: String,
}

pub(crate) async fn set_session_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<Session>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
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
    let install_target = execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        session.execution_environment,
    )
    .await
    .map_err(|err| {
        tracing::warn!(
            workspace_id = %worktree.workspace_id.0,
            "set_session_model failed to load execution settings: {err:#}",
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let adapter = crate::daemon::ensure_provider_adapter_for_target(
        state.as_ref(),
        &session.provider_id,
        install_target,
    )
    .await;

    if adapter.has_live_session(&session.id.0.to_string()).await {
        adapter
            .set_session_model(session.id.0.to_string(), req.model_id.clone())
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
    }

    let next_model_id = req.model_id.clone();

    store
        .update_session_model(session_id, next_model_id.clone())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let event = store
        .append_session_event(
            session_id,
            None,
            None,
            SessionEventType::Init,
            serde_json::json!({"current_model_id": next_model_id}),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.publish_event(event).await;

    let updated = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

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

    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
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
    let install_target = crate::execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        worktree.workspace_id,
        session.execution_environment,
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
    .await;

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

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
