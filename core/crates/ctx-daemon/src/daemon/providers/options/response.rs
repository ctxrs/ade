use ctx_core::ids::WorkspaceId;
use ctx_harness_sources as harness_sources;
use ctx_harness_sources::HarnessEndpointRecord;
use ctx_provider_runtime::provider_launch::models::endpoint_models_payload;
use ctx_providers::adapters::ProviderStatus;

mod failure;
mod finalization;
mod runtime_models;

pub(super) use self::failure::{
    config_error_provider_options_response, unusable_provider_options_response,
};
pub(super) use self::finalization::{
    finalize_provider_options_response, ProviderOptionsResponseContext,
};
pub(super) use self::runtime_models::runtime_models_provider_options_response;

fn attach_source_config(
    value: &mut serde_json::Value,
    source_config: Option<&harness_sources::HarnessProviderSourceConfig>,
) {
    if let Some(source) = source_config {
        value["source"] = serde_json::to_value(source).unwrap_or(serde_json::Value::Null);
    }
}

pub(super) struct ProviderOptionsResponseBase<'a> {
    pub(super) provider_id: &'a str,
    pub(super) workspace_id: WorkspaceId,
    pub(super) provider_status: &'a ProviderStatus,
    pub(super) has_active_auth: bool,
    pub(super) auth_mode: &'a str,
    pub(super) source_config: Option<&'a harness_sources::HarnessProviderSourceConfig>,
}

pub(super) struct ProviderOptionsProbeResult {
    pub(super) probe_ok: bool,
    pub(super) auth_required: bool,
    pub(super) probe_error: Option<String>,
}

pub(super) fn env_probe_provider_options_response(
    base: ProviderOptionsResponseBase<'_>,
    probe: ProviderOptionsProbeResult,
) -> serde_json::Value {
    let mut response = serde_json::json!({
        "provider_id": base.provider_id,
        "workspace_id": base.workspace_id.0,
        "installed": base.provider_status.installed,
        "probe_ok": probe.probe_ok,
        "supports_load": false,
        "auth_required": probe.auth_required,
        "has_active_auth": base.has_active_auth,
        "auth_mode": base.auth_mode,
        "probed_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(probe_error) = probe.probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, base.source_config);
    response
}

pub(super) fn selected_endpoint_runtime_launch_options_response(
    base: ProviderOptionsResponseBase<'_>,
    endpoint: &HarnessEndpointRecord,
    probe: ProviderOptionsProbeResult,
) -> serde_json::Value {
    let now = chrono::Utc::now();
    let mut response = serde_json::json!({
        "provider_id": base.provider_id,
        "workspace_id": base.workspace_id.0,
        "installed": base.provider_status.installed,
        "probe_ok": probe.probe_ok,
        "supports_load": false,
        "auth_required": probe.auth_required,
        "has_active_auth": base.has_active_auth,
        "auth_mode": base.auth_mode,
        "models": endpoint_models_payload(base.provider_id, endpoint, now),
        "probed_at": now.to_rfc3339(),
    });
    if let Some(probe_error) = probe.probe_error {
        response["probe_error"] = serde_json::json!(probe_error);
    }
    attach_source_config(&mut response, base.source_config);
    response
}
