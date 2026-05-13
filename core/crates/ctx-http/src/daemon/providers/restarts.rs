use std::sync::Arc;

use ctx_core::provider_ids::CODEX_PROVIDER_ID;

use crate::daemon::AppState;

pub(crate) async fn invalidate_provider_runtime_state(state: &Arc<AppState>, provider_id: &str) {
    ctx_provider_runtime::provider_cache::invalidate_provider_probe_caches(
        &state.providers,
        provider_id,
    )
    .await;
}

pub(crate) async fn restart_provider_for_auth_change(
    state: &Arc<AppState>,
    provider_id: &str,
    reason: &str,
) -> anyhow::Result<()> {
    invalidate_provider_runtime_state(state, provider_id).await;
    state
        .providers
        .drain_restart_provider_adapters_for_auth_change(provider_id, reason)
        .await
}

pub(crate) async fn restart_codex_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, CODEX_PROVIDER_ID, reason).await
}

pub(crate) async fn restart_claude_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "claude-crp", reason).await
}

pub(crate) async fn restart_gemini_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "gemini", reason).await
}

pub(crate) async fn restart_qwen_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "qwen", reason).await
}

pub(crate) async fn restart_amp_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "amp", reason).await
}

pub(crate) async fn restart_mistral_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "mistral", reason).await
}

pub(crate) async fn restart_kimi_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "kimi", reason).await
}

pub(crate) async fn restart_copilot_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "copilot", reason).await
}

pub(crate) async fn restart_cursor_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "cursor", reason).await
}
