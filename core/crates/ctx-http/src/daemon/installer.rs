mod managed_host;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::ProviderRuntimeHost;
use ctx_providers::adapters::ProviderAdapter;

pub use ctx_managed_installs::*;

pub(crate) async fn load_managed_agent_server_config_or_err(
    data_root: &Path,
) -> Result<AgentServerConfigFile> {
    ctx_managed_installs::load_agent_server_config(data_root)
        .await
        .map_err(|err| anyhow::anyhow!(ctx_observability::logs::redact_sensitive(&err.to_string())))
}

pub(crate) async fn ensure_provider_adapter_for_target_with_cfg(
    state: &impl ProviderRuntimeHost,
    cfg: &AgentServerConfigFile,
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

pub(crate) async fn ensure_provider_adapter_for_target(
    state: &impl ProviderRuntimeHost,
    provider_id: &str,
    target: InstallTarget,
) -> Result<Arc<dyn ProviderAdapter>> {
    let cfg = load_managed_agent_server_config_or_err(state.data_root()).await?;
    Ok(ensure_provider_adapter_for_target_with_cfg(state, &cfg, provider_id, target).await)
}
