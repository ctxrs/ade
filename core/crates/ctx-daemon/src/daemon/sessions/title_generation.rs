use std::path::Path as StdPath;
use std::sync::Arc;

use crate::daemon::{DaemonState, SessionsHandle};
use ctx_core::models::Session;
use ctx_observability::logs;
use ctx_provider_install::install_state::InstallId;
use ctx_session_service::title_generation;
use ctx_settings_model as user_settings;

mod persistence;
pub use ctx_managed_installs::title_generation_local::{
    TitleGenerationLocalModelStatus, TitleGenerationLocalRuntimeStatus,
};
pub use persistence::apply_session_title_update;

pub const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";

#[derive(Debug, Clone)]
pub struct TitleGenerationLocalStatusSnapshot {
    pub ready: bool,
    pub runtime: TitleGenerationLocalRuntimeStatus,
    pub model: TitleGenerationLocalModelStatus,
    pub install_id: Option<InstallId>,
    pub install_running: bool,
}

pub async fn title_generation_local_status(
    state: &Arc<DaemonState>,
) -> anyhow::Result<TitleGenerationLocalStatusSnapshot> {
    let status =
        ctx_managed_installs::title_generation_local::local_status(&state.core.data_root).await?;
    let install_id = state
        .find_running_install(TITLE_GENERATION_LOCAL_INSTALL_KEY, None)
        .await;
    Ok(TitleGenerationLocalStatusSnapshot {
        ready: status.ready,
        runtime: status.runtime,
        model: status.model,
        install_id,
        install_running: install_id.is_some(),
    })
}

pub async fn start_title_generation_local_install(state: Arc<DaemonState>) -> InstallId {
    let (install_id, started_new) = state
        .start_install(TITLE_GENERATION_LOCAL_INSTALL_KEY.to_string(), None)
        .await;
    if started_new {
        tokio::spawn(async move {
            if let Err(error) = ctx_managed_installs::install_title_generation_local_with_progress(
                state.clone(),
                install_id,
            )
            .await
            {
                tracing::error!("local title generation install failed: {error:#}");
            }
        });
    }
    install_id
}

#[derive(Debug, Clone, Copy)]
pub enum TitleGenerationSource {
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
pub struct TitleGenerationOutcome {
    pub title: String,
    pub source: TitleGenerationSource,
}

pub async fn configured_title_generation_settings(
    state: &DaemonState,
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

pub async fn generate_title_for_prompt(
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

pub async fn maybe_generate_session_title(
    state: Arc<DaemonState>,
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

impl SessionsHandle {
    pub async fn title_generation_local_status(
        &self,
    ) -> anyhow::Result<TitleGenerationLocalStatusSnapshot> {
        title_generation_local_status(&self.state).await
    }

    pub async fn start_title_generation_local_install(&self) -> InstallId {
        start_title_generation_local_install(Arc::clone(&self.state)).await
    }
}

pub async fn schedule_session_title_generation(
    state: Arc<DaemonState>,
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
