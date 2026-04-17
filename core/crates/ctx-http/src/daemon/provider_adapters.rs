#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use anyhow::Result;

#[cfg(test)]
use crate::installer;
#[cfg(test)]
use ctx_provider_install::install_state::InstallTarget;

pub(super) use ctx_provider_runtime::provider_adapters::{
    acp_status_adapter_acp_command_invalid, acp_status_adapter_bridge_invalid,
    acp_status_adapter_bridge_missing, runtime_command_as_agent_command,
    runtime_command_as_agent_command_for_target, runtime_command_invalid_adapter,
    runtime_command_missing_adapter, target_adapter_cache_key,
};

pub(crate) use ctx_provider_runtime::provider_adapters::{
    acp_bridge_adapter, acp_bridge_command, is_acp_provider_id,
};

#[cfg(test)]
pub(crate) fn runtime_probe_command_as_agent_command_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    requested_target: Option<InstallTarget>,
) -> Result<Option<installer::AgentServerCommand>> {
    crate::provider_launch::resolver::runtime_probe_command_as_agent_command_for_target(
        data_root,
        cfg,
        provider_id,
        requested_target,
    )
}

#[cfg(test)]
pub(crate) fn runtime_probe_command_as_agent_command(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
) -> Result<Option<installer::AgentServerCommand>> {
    runtime_probe_command_as_agent_command_for_target(data_root, cfg, provider_id, None)
}
