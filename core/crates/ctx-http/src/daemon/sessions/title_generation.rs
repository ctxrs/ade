use std::path::Path as StdPath;
use std::sync::Arc;

use crate::daemon::AppState;
use ctx_core::models::Session;
use ctx_observability::logs;
use ctx_session_service::title_generation;
use ctx_settings_model as user_settings;

mod persistence;
pub(crate) use persistence::apply_session_title_update;

#[derive(Debug, Clone, Copy)]
pub(crate) enum TitleGenerationSource {
    Llm,
    Fallback,
}

impl TitleGenerationSource {
    pub(super) fn as_str(self) -> &'static str {
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
    let settings = match ctx_settings_service::load_settings(state.global_store()).await {
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
