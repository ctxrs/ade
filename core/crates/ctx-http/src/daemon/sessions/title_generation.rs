use std::path::Path as StdPath;
use std::sync::Arc;

use anyhow::Context;

use crate::daemon::AppState;
use crate::settings as user_settings;
use ctx_core::models::{Session, SessionEventType};
use ctx_observability::logs;
use ctx_session_service::title_generation;

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
        state.sessions.remember_session_meta(&updated_session).await;
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
