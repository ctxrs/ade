use super::*;

pub(super) fn build_provider_adapter_for_target(
    data_root: &Path,
    cfg: &installer::AgentServerConfigFile,
    provider_id: &str,
    target: InstallTarget,
) -> Arc<dyn ProviderAdapter> {
    if matches!(provider_id, "codex" | "claude-crp") {
        return match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
            Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_provider_runtime(
                provider_id,
                cmd.command.clone(),
                cmd.args.clone(),
            )),
            Ok(None) => runtime_command_missing_adapter(provider_id),
            Err(err) => runtime_command_invalid_adapter(provider_id, err.to_string()),
        };
    }

    if is_acp_provider_id(provider_id) {
        let bridge_cmd = match runtime_command_as_agent_command_for_target(
            cfg,
            "acp-crp-bridge",
            Some(target),
        ) {
            Ok(cmd) => cmd,
            Err(err) => {
                return acp_status_adapter_bridge_invalid(
                    provider_id,
                    format!("invalid runtime command for acp-crp-bridge: {err}"),
                );
            }
        };
        let bridge_missing_message = "ACP bridge runtime is not configured".to_string();
        return match bridge_cmd.as_ref() {
            None => acp_status_adapter_bridge_missing(provider_id, bridge_missing_message),
            Some(bridge) => {
                match runtime_command_as_agent_command_for_target(cfg, provider_id, Some(target)) {
                    Ok(Some(cmd)) => {
                        match super::normalize_acp_provider_command(data_root, provider_id, cmd) {
                            Ok(cmd) => acp_bridge_adapter(provider_id, bridge, cmd),
                            Err(err) => acp_status_adapter_acp_command_invalid(
                                provider_id,
                                format!("invalid ACP command for provider '{provider_id}': {err}"),
                            ),
                        }
                    }
                    Ok(None) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("ACP command is not configured for provider '{provider_id}'"),
                    ),
                    Err(err) => acp_status_adapter_acp_command_invalid(
                        provider_id,
                        format!("invalid ACP command for provider '{provider_id}': {err}"),
                    ),
                }
            }
        };
    }

    if provider_id == "fake" {
        return Arc::new(FakeProviderAdapter::new());
    }

    runtime_command_missing_adapter(provider_id)
}
