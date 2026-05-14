use ctx_core::ids::WorkspaceId;
use ctx_harness_sources as harness_sources;
use ctx_provider_runtime::provider_usability::provider_status_unusable_reason;
use ctx_providers::adapters::ProviderStatus;

use super::attach_source_config;

pub(in crate::daemon::providers::options) fn config_error_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    installed: Option<bool>,
    auth_mode: &str,
    config_error: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = match installed {
        Some(installed) => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "installed": installed,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
        None => serde_json::json!({
            "provider_id": provider_id,
            "workspace_id": workspace_id.0,
            "probe_ok": false,
            "supports_load": false,
            "auth_required": false,
            "has_active_auth": false,
            "auth_mode": auth_mode,
            "probed_at": chrono::Utc::now().to_rfc3339(),
            "probe_error": config_error,
            "config_error": config_error,
        }),
    };
    attach_source_config(&mut response, source_config);
    response
}

pub(in crate::daemon::providers::options) fn unusable_provider_options_response(
    provider_id: &str,
    workspace_id: WorkspaceId,
    provider_status: &ProviderStatus,
    has_active_auth: bool,
    auth_mode: &str,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": provider_id,
        "workspace_id": workspace_id.0,
        "installed": provider_status.installed,
        "health": provider_status.health,
        "diagnostics": provider_status.diagnostics,
        "usability": provider_status.usability,
        "probe_ok": false,
        "probe_error": provider_status_unusable_reason(provider_status)
            .unwrap_or_else(|| "provider not ready for use".to_string()),
        "has_active_auth": has_active_auth,
        "auth_mode": auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    attach_source_config(&mut response, source_config);
    response
}
