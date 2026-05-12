use super::*;

pub(crate) async fn ensure_provider_adapter_for_target_with_cfg(
    state: &AppState,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    ctx_provider_runtime::provider_launch::resolver::ensure_provider_adapter_for_target_with_cfg(
        state,
        cfg,
        provider_id,
        target,
    )
    .await
}

pub(crate) async fn load_managed_agent_server_config_or_err(
    data_root: &Path,
) -> Result<installer::AgentServerConfigFile> {
    installer::load_agent_server_config(data_root)
        .await
        .map_err(|err| anyhow::anyhow!(ctx_observability::logs::redact_sensitive(&err.to_string())))
}

pub(crate) async fn ensure_provider_adapter_for_target(
    state: &AppState,
    provider_id: &str,
    target: InstallTarget,
) -> Result<Arc<dyn ProviderAdapter>> {
    let cfg = load_managed_agent_server_config_or_err(&state.core.data_root).await?;
    Ok(ensure_provider_adapter_for_target_with_cfg(state, &cfg, provider_id, target).await)
}
