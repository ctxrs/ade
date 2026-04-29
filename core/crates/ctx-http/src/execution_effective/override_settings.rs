use ctx_workspace_config as workspace_config;

use crate::execution_policy::{ExecutionPolicyDenied, HostExecutionPolicy};
use crate::settings::{ContainerNetworkMode, ExecutionMode, ExecutionSettings};

pub(crate) fn apply_workspace_execution_settings_override(
    settings: &mut ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    let ov = normalize_persisted_workspace_execution_settings_override(settings, ov)?;
    validate_workspace_execution_settings_override(settings, &ov)?;
    workspace_config::apply_execution_settings_override(settings, &ov);
    Ok(())
}

fn normalize_persisted_workspace_execution_settings_override(
    _settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<workspace_config::ExecutionSettingsOverride> {
    let mut normalized = ov.clone();
    // Persisted host overrides predate the daemon-owned sandbox-only gate. Treat them as stale
    // reads; new host writes still go through strict validation before persistence.
    if matches!(
        HostExecutionPolicy::current()?,
        HostExecutionPolicy::SandboxOnly
    ) && matches!(normalized.mode, Some(ExecutionMode::Host))
    {
        normalized.mode = Some(ExecutionMode::Sandbox);
        normalized.container = workspace_config::ContainerExecutionSettingsOverride::default();
    }
    Ok(normalized)
}

pub(crate) fn validate_workspace_execution_settings_override(
    settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    if matches!(settings.mode, ExecutionMode::Sandbox) {
        if matches!(ov.mode, Some(ExecutionMode::Host)) {
            return Err(ExecutionPolicyDenied::new(
                "workspace execution override cannot select host when daemon execution mode is sandbox"
            )
            .into());
        }
        validate_sandbox_network_override(settings, ov)?;
    }
    let mut effective = settings.clone();
    workspace_config::apply_execution_settings_override(&mut effective, ov);
    HostExecutionPolicy::current()?.validate_execution_settings(&effective)?;
    Ok(())
}

fn validate_sandbox_network_override(
    settings: &ExecutionSettings,
    ov: &workspace_config::ExecutionSettingsOverride,
) -> anyhow::Result<()> {
    let requested_network = ov
        .container
        .network_mode
        .as_ref()
        .unwrap_or(&settings.container.network_mode);
    match (&settings.container.network_mode, requested_network) {
        (ContainerNetworkMode::LlmOnly, ContainerNetworkMode::LlmOnly) => Ok(()),
        (ContainerNetworkMode::LlmOnly, requested) => {
            Err(ExecutionPolicyDenied::new(format!(
                "workspace execution override cannot broaden sandbox network mode from llm_only to {}",
                network_mode_label(requested)
            ))
            .into())
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::All) => {
            Err(ExecutionPolicyDenied::new(
                "workspace execution override cannot broaden sandbox network mode from allowlist to all"
            )
            .into())
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::Allowlist) => {
            validate_allowlist_subset(&settings.container.allowlist, &ov.container.allowlist)
        }
        (ContainerNetworkMode::Allowlist, ContainerNetworkMode::LlmOnly) => Ok(()),
        (ContainerNetworkMode::All, _) => Ok(()),
    }
}

fn validate_allowlist_subset(
    daemon_allowlist: &[String],
    workspace_allowlist: &Option<Vec<String>>,
) -> anyhow::Result<()> {
    let Some(workspace_allowlist) = workspace_allowlist else {
        return Ok(());
    };
    let allowed = daemon_allowlist
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
        .collect::<std::collections::BTreeSet<_>>();
    for entry in workspace_allowlist
        .iter()
        .filter_map(|value| trimmed_nonempty(value))
    {
        if !allowed.contains(&entry) {
            return Err(ExecutionPolicyDenied::new(format!(
                "workspace execution allowlist entry `{entry}` is not allowed by daemon sandbox allowlist"
            ))
            .into());
        }
    }
    Ok(())
}

fn trimmed_nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn network_mode_label(mode: &ContainerNetworkMode) -> &'static str {
    match mode {
        ContainerNetworkMode::LlmOnly => "llm_only",
        ContainerNetworkMode::Allowlist => "allowlist",
        ContainerNetworkMode::All => "all",
    }
}
