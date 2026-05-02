use super::*;

pub(super) async fn invalidate_provider_runtime_state(state: &Arc<AppState>, provider_id: &str) {
    probe::invalidate_provider_probe_caches(state, provider_id).await;
}

pub(super) async fn restart_provider_for_auth_change(
    state: &Arc<AppState>,
    provider_id: &str,
    reason: &str,
) -> anyhow::Result<()> {
    invalidate_provider_runtime_state(state, provider_id).await;
    let target_prefix = format!("{provider_id}@");
    let mut adapters = {
        let map = state.providers.adapters.lock().await;
        [provider_id]
            .iter()
            .filter_map(|id| {
                map.get(*id)
                    .map(|adapter| (id.to_string(), Arc::clone(adapter)))
            })
            .collect::<Vec<_>>()
    };
    let target_adapters = {
        let map = state.providers.target_adapters.lock().await;
        map.iter()
            .filter(|(id, _)| id.starts_with(&target_prefix))
            .map(|(id, adapter)| (id.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>()
    };
    adapters.extend(target_adapters);
    let mut failures = Vec::new();
    for (id, adapter) in adapters {
        if !adapter.supports_restart_mode(ProviderRestartMode::Drain) {
            tracing::info!("skipping drain-restart for {id} after auth change: adapter does not support drain restart");
            continue;
        }
        if let Err(err) = adapter.restart(reason, ProviderRestartMode::Drain).await {
            tracing::warn!("failed to drain-restart {id} after auth change: {err}");
            failures.push(format!("{id}: {err:#}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "provider auth updated but drain-restart failed for {provider_id}: {}",
            failures.join("; ")
        );
    }
}

pub(super) async fn restart_codex_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, CODEX_PROVIDER_ID, reason).await
}

pub(super) async fn restart_claude_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "claude-crp", reason).await
}

pub(super) async fn restart_gemini_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "gemini", reason).await
}

pub(super) async fn restart_qwen_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "qwen", reason).await
}

pub(super) async fn restart_amp_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "amp", reason).await
}

pub(super) async fn restart_mistral_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "mistral", reason).await
}

pub(super) async fn restart_kimi_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "kimi", reason).await
}

pub(super) async fn restart_copilot_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "copilot", reason).await
}

pub(super) async fn restart_cursor_providers_for_auth_change(
    state: &Arc<AppState>,
    reason: &str,
) -> anyhow::Result<()> {
    restart_provider_for_auth_change(state, "cursor", reason).await
}
