use super::*;

mod adapter_factory;

use adapter_factory::build_provider_adapter_for_target;

pub(crate) async fn ensure_provider_adapter_for_target_with_cfg(
    state: &AppState,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if let Some(cache_key) = target_adapter_cache_key(provider_id, target) {
        if let Some(adapter) = state
            .providers
            .target_adapters
            .lock()
            .await
            .get(&cache_key)
            .cloned()
        {
            return adapter;
        }
        let adapter =
            build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
        state
            .providers
            .target_adapters
            .lock()
            .await
            .insert(cache_key, adapter.clone());
        return adapter;
    }

    if let Some(adapter) = state
        .providers
        .adapters
        .lock()
        .await
        .get(provider_id)
        .cloned()
    {
        return adapter;
    }
    let adapter =
        build_provider_adapter_for_target(&state.core.data_root, cfg, provider_id, target);
    state
        .providers
        .adapters
        .lock()
        .await
        .insert(provider_id.to_string(), adapter.clone());
    adapter
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

pub(crate) fn normalize_acp_provider_command(
    data_root: &Path,
    provider_id: &str,
    cmd: installer::AgentServerCommand,
) -> Result<installer::AgentServerCommand> {
    ctx_provider_runtime::provider_launch::resolver::normalize_acp_provider_command(
        data_root,
        provider_id,
        cmd,
    )
}
