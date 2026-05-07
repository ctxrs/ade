use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_providers::adapters::ProviderAdapter;
use ctx_providers::crp::Tier1CrpAdapter;
use ctx_providers::fake::FakeProviderAdapter;

use crate::installer;

use super::provider_adapters::{
    acp_bridge_adapter, acp_status_adapter_acp_command_invalid, acp_status_adapter_bridge_invalid,
    acp_status_adapter_bridge_missing, runtime_command_as_agent_command,
    runtime_command_invalid_adapter, runtime_command_missing_adapter,
};
use super::provider_bootstrap::normalize_acp_provider_command;

pub(super) fn build_startup_provider_adapters(
    data_root: &Path,
    agent_cfg: &installer::AgentServerConfigFile,
) -> HashMap<String, Arc<dyn ProviderAdapter>> {
    let mut bridge_runtime_error: Option<String> = None;
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    let bridge_cmd = match runtime_command_as_agent_command(agent_cfg, "acp-crp-bridge") {
        Ok(cmd) => cmd,
        Err(err) => {
            let message = format!("invalid runtime command for acp-crp-bridge: {err}");
            tracing::warn!("{message}");
            bridge_runtime_error = Some(message);
            None
        }
    };

    for provider_id in ["codex", "claude-crp"] {
        let adapter: Arc<dyn ProviderAdapter> =
            match runtime_command_as_agent_command(agent_cfg, provider_id) {
                Ok(Some(cmd)) => Arc::new(Tier1CrpAdapter::from_provider_runtime(
                    provider_id,
                    cmd.command.clone(),
                    cmd.args.clone(),
                )),
                Ok(None) => runtime_command_missing_adapter(provider_id),
                Err(err) => runtime_command_invalid_adapter(provider_id, err.to_string()),
            };
        providers.insert(provider_id.to_string(), adapter);
    }

    for provider_id in [
        "gemini",
        "qwen",
        "cursor",
        "pi",
        "opencode",
        "mistral",
        "goose",
        "kimi",
        "auggie",
        "amp",
        "droid",
        "copilot",
        "cline",
        "openhands",
    ] {
        let bridge_missing_message = bridge_runtime_error
            .clone()
            .unwrap_or_else(|| "ACP bridge runtime is not configured".to_string());
        let adapter = match bridge_cmd.as_ref() {
            None => {
                if let Some(message) = bridge_runtime_error.clone() {
                    acp_status_adapter_bridge_invalid(provider_id, message)
                } else {
                    acp_status_adapter_bridge_missing(provider_id, bridge_missing_message)
                }
            }
            Some(bridge) => match runtime_command_as_agent_command(agent_cfg, provider_id) {
                Ok(Some(cmd)) => {
                    match normalize_acp_provider_command(data_root, provider_id, cmd) {
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
            },
        };
        providers.insert(provider_id.to_string(), adapter);
    }

    // codex/claude CRP adapters are always registered now (legacy ACP bridge removed).

    if std::env::var("CTX_SHOW_FAKE_PROVIDER")
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
        .unwrap_or(false)
    {
        providers.insert("fake".into(), Arc::new(FakeProviderAdapter::new()));
    }

    providers
}
