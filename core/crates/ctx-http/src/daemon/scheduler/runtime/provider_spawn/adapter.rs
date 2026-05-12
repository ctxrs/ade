use std::sync::Arc;

use anyhow::{anyhow, Result};
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::ProviderAdapter;

use crate::daemon::installer;
use crate::daemon::AppState;

pub(in crate::daemon::scheduler::runtime) struct PreparedProviderAdapter {
    pub(in crate::daemon::scheduler::runtime) adapter: Arc<dyn ProviderAdapter>,
    pub(in crate::daemon::scheduler::runtime) adapter_cfg: installer::AgentServerConfigFile,
    pub(in crate::daemon::scheduler::runtime) install_target: InstallTarget,
}

pub(in crate::daemon::scheduler::runtime) async fn prepare_provider_adapter_for_turn(
    state: &Arc<AppState>,
    runtime_provider_id: &str,
    is_linux_sandbox: bool,
) -> Result<PreparedProviderAdapter> {
    let install_target = provider_install_target_for_runtime(is_linux_sandbox);
    let adapter_cfg = installer::load_managed_agent_server_config_or_err(&state.core.data_root)
        .await
        .map_err(|err| anyhow!(err.to_string()))?;
    let adapter = installer::ensure_provider_adapter_for_target_with_cfg(
        state.as_ref(),
        &adapter_cfg,
        runtime_provider_id,
        install_target,
    )
    .await;

    Ok(PreparedProviderAdapter {
        adapter,
        adapter_cfg,
        install_target,
    })
}

fn provider_install_target_for_runtime(is_linux_sandbox: bool) -> InstallTarget {
    if is_linux_sandbox {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_sandbox_runtime_uses_container_install_target() {
        assert_eq!(
            provider_install_target_for_runtime(true),
            InstallTarget::Container
        );
    }

    #[test]
    fn host_runtime_uses_host_install_target() {
        assert_eq!(
            provider_install_target_for_runtime(false),
            InstallTarget::Host
        );
    }
}
